//! Differential test of [`reconstruct_remote`] against a model of the remote volume (#456).
//!
//! The oracle is what a full walk would list: the model keeps the real tree, generates a random
//! history of changes to it, turns that history into the event delta the stream would carry, and
//! hands [`reconstruct_remote`] the baseline from before the history and a resolver that answers
//! listings from the tree **as it is at the end**, as the CLI does. A `Complete` map must equal the
//! filtered walk; a snapshot fallback is always acceptable (it is the old behaviour), and a test
//! fails only on a `Complete` map that is **wrong**: a change the pass would lose while the cursor
//! moved past it.
//!
//! Everything is seeded and deterministic. The default run is a few thousand scenarios per
//! configuration and takes about a second; the large run is the same code with more seeds, and
//! `RECONSTRUCT_FUZZ_FROM` starts at a given seed (to replay a failure by its number):
//!
//! ```bash
//! RECONSTRUCT_FUZZ_SCENARIOS=300000 cargo test --lib reconstruct::model_tests -- --nocapture
//! RECONSTRUCT_FUZZ_FROM=215742 RECONSTRUCT_FUZZ_SCENARIOS=1 cargo test --lib \
//!   reconstruct::model_tests::two_passes -- --nocapture
//! ```
//!
//! ## What the model deliberately leaves out
//!
//! Each of these is a limit of volume events that is older than #456 and equally present when the
//! pair does not know its root (the "control" runs the review used), so it is not generated rather
//! than reported:
//!
//! * a directory **with descendants** is never renamed or moved *within* the tree (nor out and back
//!   again in one history, which is the same move): events are per link, so nothing re-keys the
//!   descendants already in the map;
//! * a folder **that has no index row** is never trashed, deleted, restored or moved into the tree
//!   while it has descendants: the event is a single one for the folder's own link and the pair,
//!   which does not track the folder, cannot tell which of its records are under it. (Include
//!   rules and indexes written before folders were rows both produce such folders.) A pair with
//!   include rules does not use the root uid at all, so these cases are decided exactly as before
//!   #456 and by the same code;
//! * nothing is created in, renamed in, restored into, or moved into a folder that has no row
//!   **and nothing recorded beneath it** by a node the baseline does not hold (the one hole the ADR
//!   names: such a folder is invisible to the index, so an event inside it reads as somewhere else
//!   on the volume). A node the baseline holds *is* moved into one, because that is the case the
//!   pair must walk for;
//! * the two-pass check does not compare a history whose second pass trashes, deletes or moves out
//!   of the tree a node the first pass overwrote: that pass reads its events against a listing
//!   already ahead of them, so it can place a node at a path whose previous node is only trashed
//!   (or moved out) after the cut, and that event then reads as somebody else's (the old node's
//!   children stay in the map). The code before #456 overwrote the record the same way; a walk
//!   heals it. Replay with seed 215742 (a trash) or 91177341 (a move out) and this skip removed;
//! * a record under an id of an older index is not renamed, nor moved within the tree: the id
//!   matches no event, so the old path stays in the map until a walk replaces the record. (Moving
//!   it out of the tree, trashing it or deleting it *are* generated.)

use super::*;
use crate::index::SyncStatus;

const VOLUME: &str = "vol";
/// Scenarios per test when `RECONSTRUCT_FUZZ_SCENARIOS` is not set.
const DEFAULT_SCENARIOS: u64 = 10_000;
/// Node 1 is the pair's remote root.
const ROOT_NODE: usize = 1;
const ROOT_UID: &str = "vol~n1";

fn scenarios(default: u64) -> u64 {
    std::env::var("RECONSTRUCT_FUZZ_SCENARIOS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// The first seed of a run: `RECONSTRUCT_FUZZ_FROM`, default 0.
fn first_seed() -> u64 {
    std::env::var("RECONSTRUCT_FUZZ_FROM")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        let mut rng = Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        for _ in 0..4 {
            rng.next();
        }
        rng
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// The selective-sync configuration a scenario runs under.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rules {
    None,
    /// `exclude b/**`
    ExcludeB,
    /// `include **/a`: folders on the way to a match are not entities.
    IncludeAnyA,
    /// `include b/**`
    IncludeUnderB,
    /// `include a/**` and `**/c`
    IncludeUnderAOrAnyC,
}

impl Rules {
    fn options(self) -> ScanOptions {
        let (include, exclude): (&[&str], &[&str]) = match self {
            Self::None => (&[], &[]),
            Self::ExcludeB => (&[], &["b/**"]),
            Self::IncludeAnyA => (&["**/a"], &[]),
            Self::IncludeUnderB => (&["b/**"], &[]),
            Self::IncludeUnderAOrAnyC => (&["a/**", "**/c"], &[]),
        };
        let own = |patterns: &[&str]| -> Vec<String> {
            patterns.iter().map(|p| (*p).to_owned()).collect()
        };
        ScanOptions::new(
            Path::new("/root"),
            &[],
            &own(include),
            &own(exclude),
            &crate::sync::ConflictNaming::default(),
        )
        .expect("scan options")
    }
}

#[derive(Clone, Debug)]
struct Node {
    name: String,
    parent: Option<usize>,
    dir: bool,
    trashed: bool,
    deleted: bool,
    sha: u32,
}

/// The remote volume: node 0 is the volume's top, 1 the pair's root, 2 a sibling folder outside it.
#[derive(Clone)]
struct World {
    nodes: Vec<Node>,
    /// Whether an event for a child of the volume's top names that parent (the stream may omit it).
    top_ids: bool,
}

/// What a full walk lists: `(is_directory, uid, digest)` by root-relative path.
type Truth = HashMap<PathBuf, (bool, String, String)>;

impl World {
    fn live(&self, i: usize) -> bool {
        !self.nodes[i].trashed && !self.nodes[i].deleted
    }

    fn chain_live(&self, mut i: usize) -> bool {
        loop {
            if !self.live(i) {
                return false;
            }
            match self.nodes[i].parent {
                Some(parent) => i = parent,
                None => return true,
            }
        }
    }

    fn inside(&self, mut i: usize) -> bool {
        loop {
            if i == ROOT_NODE {
                return true;
            }
            match self.nodes[i].parent {
                Some(parent) => i = parent,
                None => return false,
            }
        }
    }

    fn live_children(&self, parent: usize) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&i| self.nodes[i].parent == Some(parent) && self.live(i))
            .collect()
    }

    fn rel_path(&self, i: usize) -> PathBuf {
        let mut parts = Vec::new();
        let mut current = i;
        while current != ROOT_NODE {
            parts.push(self.nodes[current].name.clone());
            current = self.nodes[current].parent.expect("an inside node");
        }
        parts.reverse();
        parts.iter().collect()
    }

    /// The live node at a root-relative path, as a listing would resolve it.
    fn find(&self, relative: &Path) -> Option<usize> {
        if !self.chain_live(ROOT_NODE) {
            return None;
        }
        let mut current = ROOT_NODE;
        for component in relative.components() {
            let name = component.as_os_str().to_string_lossy().into_owned();
            current = self
                .live_children(current)
                .into_iter()
                .find(|&child| self.nodes[child].name == name)?;
        }
        Some(current)
    }

    fn entity(&self, i: usize, path: PathBuf) -> RemoteEntity {
        let node = &self.nodes[i];
        if node.dir {
            RemoteEntity::Directory(RemoteDirectory {
                path,
                id: Some(format!("vol~n{i}")),
                name: node.name.clone(),
            })
        } else {
            RemoteEntity::File(RemoteFile {
                path,
                id: format!("vol~n{i}"),
                name: node.name.clone(),
                sha1_hash: Some(format!("h{}", node.sha)),
                downloadable: true,
            })
        }
    }

    fn event_parent(&self, i: usize) -> Option<String> {
        match self.nodes[i].parent {
            Some(0) if !self.top_ids => None,
            Some(parent) => Some(format!("n{parent}")),
            None => None,
        }
    }

    fn event(
        &self,
        kind: RemoteChangeKind,
        i: usize,
        trashed: bool,
        number: usize,
    ) -> RemoteChange {
        RemoteChange {
            kind,
            node_id: format!("n{i}"),
            parent_id: match kind {
                RemoteChangeKind::Deleted => None,
                _ => self.event_parent(i),
            },
            trashed,
            shared: false,
            event_id: format!("e{number}"),
        }
    }
}

/// The CLI's view at the end of the history: it lists current state, whatever the events say.
struct ModelResolver {
    world: World,
    indexed: HashMap<String, PathBuf>,
}

impl RemoteChangeResolver for ModelResolver {
    fn indexed_path(&self, uid: &str) -> AppResult<Option<PathBuf>> {
        Ok(self.indexed.get(uid).cloned())
    }

    fn list(&self, dir: &Path) -> AppResult<Rc<HashMap<PathBuf, RemoteEntity>>> {
        let Some(parent) = self.world.find(dir) else {
            return Err(crate::boxed_error("Node not found"));
        };
        if !self.world.nodes[parent].dir {
            return Err(crate::boxed_error("not a directory"));
        }
        let mut listing = HashMap::new();
        for child in self.world.live_children(parent) {
            let path = dir.join(&self.world.nodes[child].name);
            listing.insert(path.clone(), self.world.entity(child, path));
        }
        Ok(Rc::new(listing))
    }
}

fn record_of(world: &World, i: usize, with_id: bool) -> FileRecord {
    let node = &world.nodes[i];
    FileRecord {
        file_path: world.rel_path(i),
        entity_kind: if node.dir {
            EntityKind::Directory
        } else {
            EntityKind::File
        },
        file_size: 1,
        mtime: 0,
        sha1_hash: (!node.dir).then(|| format!("h{}", node.sha)),
        proton_id: with_id.then(|| format!("vol~n{i}")),
        sync_status: SyncStatus::Synced,
    }
}

struct Scenario {
    base: HashMap<PathBuf, FileRecord>,
    events: Vec<RemoteChange>,
    world: World,
    /// The unfiltered index's uid -> path answers.
    indexed: HashMap<String, PathBuf>,
    log: Vec<String>,
    /// Folder rows the baseline lacks on purpose: a pass without an event for them cannot know
    /// them, and the next walk adds them, so the comparison leaves them out on both sides.
    dropped_rows: HashSet<PathBuf>,
    /// Records the baseline holds under an id of an older index: path -> (node, that id). The map
    /// keeps such an id until a walk replaces it, so the comparison does not ask for the composed
    /// one while the record is untouched.
    legacy: HashMap<PathBuf, (usize, String)>,
}

/// Another node takes `name` under `parent`, when the folder is live, the name is free and the
/// folder is not one a pair cannot see into.
fn retake_name(
    world: &mut World,
    events: &mut Vec<RemoteChange>,
    log: &mut Vec<String>,
    rng: &mut Rng,
    holes: &HashSet<usize>,
    parent: usize,
    name: &str,
) {
    if !world.chain_live(parent)
        || holes.contains(&parent)
        || world
            .live_children(parent)
            .iter()
            .any(|&child| world.nodes[child].name == name)
    {
        return;
    }
    let j = world.nodes.len();
    world.nodes.push(Node {
        name: name.to_owned(),
        parent: Some(parent),
        dir: rng.chance(50),
        trashed: false,
        deleted: false,
        sha: 1,
    });
    log.push(format!("retake {name} as node {j} under node {parent}"));
    events.push(world.event(RemoteChangeKind::Created, j, false, events.len()));
}

#[derive(Clone, Copy)]
struct Shape {
    rules: Rules,
    max_ops: usize,
    /// Some folders of the baseline have no row (an index written before folders were rows).
    drop_folder_rows: bool,
    /// Also folders that hold nothing lose their row. Nothing is generated *into* such a folder
    /// by a node the pair does not hold already (the one hole the ADR names); a node it does hold
    /// may move into one, which is the case this shape is for.
    drop_empty_folder_rows: bool,
    /// Percent: after a node leaves a folder (moved out, trashed, deleted), another node takes
    /// its name there.
    retake_names: u64,
    /// Percent: a trash is undone at once.
    quick_restore: u64,
    /// A live node can get a `Deleted` event without ever being trashed.
    direct_delete: bool,
    /// Percent of created/updated events that lose their parent id.
    parentless: u64,
    /// Percent of baseline records that carry an id of an older index (a raw id, or another
    /// volume's) instead of a composed one of this volume.
    legacy_ids: u64,
    /// A folder with no row may be trashed or deleted while it holds records. Only a pair that
    /// knows its root reads that (a removal it does not hold, with a record in a folder no record
    /// holds, walks), so a run with include rules leaves it off.
    trash_rowless_folders: bool,
}

fn allowed(options: &ScanOptions, world: &World, i: usize) -> bool {
    let path = world.rel_path(i);
    if world.nodes[i].dir {
        options.allows_relative_directory(&path)
    } else {
        options.allows_relative_file(&path)
    }
}

fn generate(seed: u64, shape: Shape, options: &ScanOptions) -> Scenario {
    let mut rng = Rng::new(seed);
    let node = |name: &str, parent: Option<usize>| Node {
        name: name.to_owned(),
        parent,
        dir: true,
        trashed: false,
        deleted: false,
        sha: 0,
    };
    let mut world = World {
        nodes: vec![node("top", None), node("R", Some(0)), node("O", Some(0))],
        top_ids: rng.chance(50),
    };
    let names = ["a", "b", "c"];
    let mut log = Vec::new();

    // Nodes that existed before the baseline (their events were consumed before the cursor).
    for _ in 0..rng.below(6) {
        let dirs: Vec<usize> = (1..world.nodes.len())
            .filter(|&i| world.nodes[i].dir && world.live(i))
            .collect();
        let parent = dirs[rng.below(dirs.len())];
        let name = names[rng.below(3)].to_owned();
        if world
            .live_children(parent)
            .iter()
            .any(|&child| world.nodes[child].name == name)
        {
            continue;
        }
        world.nodes.push(Node {
            name,
            parent: Some(parent),
            dir: rng.chance(40),
            trashed: false,
            deleted: false,
            sha: 1,
        });
    }

    let mut base: HashMap<PathBuf, FileRecord> = HashMap::new();
    let mut in_base: HashSet<usize> = HashSet::new();
    // Folders that are not entities (an include rule) or whose row was dropped have no row.
    let mut rowless: HashSet<usize> = HashSet::new();
    let mut dropped_rows: HashSet<PathBuf> = HashSet::new();
    let mut legacy: HashMap<PathBuf, (usize, String)> = HashMap::new();
    for i in 0..world.nodes.len() {
        if i == ROOT_NODE || !world.inside(i) || !world.chain_live(i) {
            continue;
        }
        if !allowed(options, &world, i) {
            if world.nodes[i].dir {
                rowless.insert(i);
            }
            continue;
        }
        let mut record = record_of(&world, i, true);
        if shape.legacy_ids > 0 && rng.chance(shape.legacy_ids) {
            let id = if rng.chance(50) {
                format!("n{i}")
            } else {
                format!("other~n{i}")
            };
            record.proton_id = Some(id.clone());
            legacy.insert(world.rel_path(i), (i, id));
        }
        base.insert(world.rel_path(i), record);
        in_base.insert(i);
    }
    // Folders a pair cannot see into: no row, and no record beneath to say they exist.
    let mut holes: HashSet<usize> = HashSet::new();
    if shape.drop_folder_rows || shape.drop_empty_folder_rows {
        // A folder with a record beneath it keeps the evidence the index gives that it exists. One
        // with no row and nothing beneath it is invisible to the index (it arises when rules are
        // loosened, and a `resync` heals it); see the ADR.
        let mut folders: Vec<usize> = in_base
            .iter()
            .copied()
            .filter(|&i| world.nodes[i].dir)
            .collect();
        folders.sort_unstable();
        let mut dropped_nodes = Vec::new();
        for i in folders {
            let path = world.rel_path(i);
            let holds_a_record = base
                .keys()
                .any(|other| other != &path && other.starts_with(&path));
            let droppable = if holds_a_record {
                shape.drop_folder_rows
            } else {
                shape.drop_empty_folder_rows
            };
            if droppable && rng.chance(40) {
                base.remove(&path);
                in_base.remove(&i);
                rowless.insert(i);
                dropped_rows.insert(path);
                dropped_nodes.push(i);
            }
        }
        for i in dropped_nodes {
            let path = world.rel_path(i);
            if !base
                .keys()
                .any(|other| other != &path && other.starts_with(&path))
            {
                holes.insert(i);
            }
        }
    }
    let index_of = |base: &HashMap<PathBuf, FileRecord>| -> HashMap<String, PathBuf> {
        base.iter()
            .filter_map(|(path, record)| Some((record.proton_id.clone()?, path.clone())))
            .collect()
    };
    let indexed = index_of(&base);

    let mut events: Vec<RemoteChange> = Vec::new();
    // Once something structural happened inside the tree, no further record is "just uploaded".
    let mut structural_inside = false;
    // Directories that left the tree holding something in this history.
    let mut left_the_tree_with_children: HashSet<usize> = HashSet::new();
    let mut sha_counter = 10;
    // Folders that had no row while they were in the tree, at any point of the history. A folder
    // can leave with its parent, be trashed there, and come back with the parent: it is the same
    // folder with the same records, wherever it is when the event arrives.
    let mut rowless_while_inside: HashSet<usize> = HashSet::new();
    let has_daemon_made_record = |base: &HashMap<PathBuf, FileRecord>, world: &World, i: usize| {
        world.inside(i)
            && base
                .values()
                .any(|r| r.proton_id.is_none() && world.rel_path(i) == r.file_path)
    };

    for _ in 0..(1 + rng.below(shape.max_ops)) {
        for i in 0..world.nodes.len() {
            if i != ROOT_NODE
                && world.nodes[i].dir
                && world.inside(i)
                && (rowless.contains(&i) || !allowed(options, &world, i))
            {
                rowless_while_inside.insert(i);
            }
        }
        let live_dirs: Vec<usize> = (0..world.nodes.len())
            .filter(|&i| world.nodes[i].dir && world.chain_live(i))
            .collect();
        let live_nodes: Vec<usize> = (3..world.nodes.len())
            .filter(|&i| world.chain_live(i))
            .collect();
        match rng.below(10) {
            // create
            0..=3 => {
                let parent = live_dirs[rng.below(live_dirs.len())];
                let name = names[rng.below(3)].to_owned();
                if world
                    .live_children(parent)
                    .iter()
                    .any(|&child| world.nodes[child].name == name)
                    || holes.contains(&parent)
                {
                    continue;
                }
                // The daemon's own upload or mkdir, whose record has no id until its Created
                // event arrives in this delta.
                let daemon_made = world.inside(parent)
                    && (in_base.contains(&parent) || parent == ROOT_NODE)
                    && !structural_inside
                    && rng.chance(40);
                let dir = rng.chance(45);
                let i = world.nodes.len();
                world.nodes.push(Node {
                    name: name.clone(),
                    parent: Some(parent),
                    dir,
                    trashed: false,
                    deleted: false,
                    sha: 1,
                });
                let daemon_made = daemon_made && allowed(options, &world, i);
                if daemon_made {
                    base.insert(world.rel_path(i), record_of(&world, i, false));
                    in_base.insert(i);
                }
                log.push(format!(
                    "create {} {name} under {} ({}){}",
                    if dir { "dir" } else { "file" },
                    world.nodes[parent].name,
                    if world.inside(parent) {
                        "inside"
                    } else {
                        "outside"
                    },
                    if daemon_made { " [daemon-made]" } else { "" }
                ));
                events.push(world.event(RemoteChangeKind::Created, i, false, events.len()));
            }
            // modify a file
            4 => {
                let files: Vec<usize> = live_nodes
                    .iter()
                    .copied()
                    .filter(|&i| !world.nodes[i].dir)
                    .collect();
                if files.is_empty() {
                    continue;
                }
                let i = files[rng.below(files.len())];
                if has_daemon_made_record(&base, &world, i) {
                    continue;
                }
                sha_counter += 1;
                world.nodes[i].sha = sha_counter;
                structural_inside |= world.inside(i);
                log.push(format!("modify file node {i}"));
                events.push(world.event(RemoteChangeKind::Updated, i, false, events.len()));
            }
            // rename a file or an empty directory
            5 => {
                if live_nodes.is_empty() {
                    continue;
                }
                let i = live_nodes[rng.below(live_nodes.len())];
                let name = names[rng.below(3)].to_owned();
                let parent = world.nodes[i].parent.expect("a parent");
                if world.nodes[i].dir && !world.live_children(i).is_empty() {
                    continue;
                }
                // A record under an id of an older index is not re-keyed by a rename (nor by a
                // move within the tree): the walk is what replaces such an id.
                if world
                    .live_children(parent)
                    .iter()
                    .any(|&child| world.nodes[child].name == name)
                    || has_daemon_made_record(&base, &world, i)
                    || legacy.values().any(|(node, _)| *node == i)
                    // A folder inside a folder the pair cannot see into is invisible too.
                    || holes.contains(&parent)
                {
                    continue;
                }
                structural_inside |= world.inside(i);
                world.nodes[i].name = name.clone();
                log.push(format!("rename node {i} -> {name}"));
                events.push(world.event(RemoteChangeKind::Updated, i, false, events.len()));
            }
            // move
            6 | 7 => {
                if live_nodes.is_empty() {
                    continue;
                }
                let i = live_nodes[rng.below(live_nodes.len())];
                let target = live_dirs[rng.below(live_dirs.len())];
                let (was_inside, now_inside) = (world.inside(i), world.inside(target));
                // never into itself or its own subtree
                let mut current = Some(target);
                let mut cyclic = false;
                while let Some(c) = current {
                    if c == i {
                        cyclic = true;
                        break;
                    }
                    current = world.nodes[c].parent;
                }
                let name = world.nodes[i].name.clone();
                if cyclic
                    || world.nodes[i].parent == Some(target)
                    || world
                        .live_children(target)
                        .iter()
                        .any(|&child| world.nodes[child].name == name)
                    || has_daemon_made_record(&base, &world, i)
                    // See the module doc: nothing a pair does not hold already moves into a folder
                    // it cannot see into, and a record under an older id does not move within.
                    || (holes.contains(&target) && !in_base.contains(&i))
                    || (was_inside
                        && now_inside
                        && legacy.values().any(|(node, _)| *node == i))
                {
                    continue;
                }
                // See the module doc: a directory with descendants is not moved within the tree,
                // and a folder without a row is not moved into it while it holds anything.
                // Out and back again is a move within the tree, so it is left out too.
                if world.nodes[i].dir && !world.live_children(i).is_empty() {
                    let lands_as_an_entity = now_inside
                        && options.allows_relative_directory(&world.rel_path(target).join(&name));
                    if (was_inside && now_inside)
                        || (now_inside && !lands_as_an_entity)
                        || (now_inside && left_the_tree_with_children.contains(&i))
                    {
                        continue;
                    }
                    if was_inside && !now_inside {
                        left_the_tree_with_children.insert(i);
                    }
                }
                let old_parent = world.nodes[i].parent.expect("a parent");
                world.nodes[i].parent = Some(target);
                structural_inside |= was_inside || now_inside;
                log.push(format!(
                    "move {} node {i} ({name}) {} -> {} under node {target}",
                    if world.nodes[i].dir { "dir" } else { "file" },
                    if was_inside { "inside" } else { "outside" },
                    if now_inside { "inside" } else { "outside" },
                ));
                events.push(world.event(RemoteChangeKind::Updated, i, false, events.len()));
                if shape.retake_names > 0 && rng.chance(shape.retake_names) {
                    retake_name(
                        &mut world,
                        &mut events,
                        &mut log,
                        &mut rng,
                        &holes,
                        old_parent,
                        &name,
                    );
                }
            }
            // trash
            8 => {
                if live_nodes.is_empty() {
                    continue;
                }
                let i = live_nodes[rng.below(live_nodes.len())];
                if !shape.trash_rowless_folders
                    && row_less_with_descendants(&world, &rowless_while_inside, i)
                {
                    continue;
                }
                let name = world.nodes[i].name.clone();
                let parent = world.nodes[i].parent.expect("a parent");
                structural_inside |= world.inside(i);
                world.nodes[i].trashed = true;
                log.push(format!("trash node {i}"));
                events.push(world.event(RemoteChangeKind::Updated, i, true, events.len()));
                if shape.quick_restore > 0
                    && rng.chance(shape.quick_restore)
                    && !holes.contains(&parent)
                {
                    world.nodes[i].trashed = false;
                    log.push(format!("restore node {i} at once"));
                    events.push(world.event(RemoteChangeKind::Updated, i, false, events.len()));
                } else if shape.retake_names > 0 && rng.chance(shape.retake_names) {
                    retake_name(
                        &mut world,
                        &mut events,
                        &mut log,
                        &mut rng,
                        &holes,
                        parent,
                        &name,
                    );
                }
            }
            // restore a trashed node, or delete it for good
            _ => {
                if shape.direct_delete && rng.chance(25) && !live_nodes.is_empty() {
                    // Deleted without a trash first.
                    let i = live_nodes[rng.below(live_nodes.len())];
                    if shape.trash_rowless_folders
                        || !row_less_with_descendants(&world, &rowless_while_inside, i)
                    {
                        let name = world.nodes[i].name.clone();
                        let parent = world.nodes[i].parent.expect("a parent");
                        structural_inside |= world.inside(i);
                        world.nodes[i].deleted = true;
                        log.push(format!("delete live node {i}"));
                        events.push(world.event(RemoteChangeKind::Deleted, i, false, events.len()));
                        if shape.retake_names > 0 && rng.chance(shape.retake_names) {
                            retake_name(
                                &mut world,
                                &mut events,
                                &mut log,
                                &mut rng,
                                &holes,
                                parent,
                                &name,
                            );
                        }
                    }
                    continue;
                }
                let trashed: Vec<usize> = (3..world.nodes.len())
                    .filter(|&i| {
                        world.nodes[i].trashed
                            && !world.nodes[i].deleted
                            && world.nodes[i].parent.is_some_and(|p| world.chain_live(p))
                    })
                    .collect();
                if trashed.is_empty() {
                    continue;
                }
                let i = trashed[rng.below(trashed.len())];
                let name = world.nodes[i].name.clone();
                let parent = world.nodes[i].parent.expect("a parent");
                if rng.chance(60) {
                    if world
                        .live_children(parent)
                        .iter()
                        .any(|&child| world.nodes[child].name == name)
                        || row_less_with_descendants(&world, &rowless_while_inside, i)
                        || holes.contains(&parent)
                    {
                        continue;
                    }
                    world.nodes[i].trashed = false;
                    structural_inside |= world.inside(i);
                    log.push(format!("restore node {i} ({name})"));
                    events.push(world.event(RemoteChangeKind::Updated, i, false, events.len()));
                } else {
                    world.nodes[i].deleted = true;
                    log.push(format!("delete node {i} ({name}) for good"));
                    events.push(world.event(RemoteChangeKind::Deleted, i, false, events.len()));
                    if shape.retake_names > 0 && rng.chance(shape.retake_names) {
                        retake_name(
                            &mut world,
                            &mut events,
                            &mut log,
                            &mut rng,
                            &holes,
                            parent,
                            &name,
                        );
                    }
                }
            }
        }
    }
    if shape.parentless > 0 {
        // The stream may leave a parent out; the pair must not read that as "somewhere else".
        for event in &mut events {
            if !matches!(event.kind, RemoteChangeKind::Deleted)
                && event.parent_id.is_some()
                && rng.chance(shape.parentless)
            {
                event.parent_id = None;
            }
        }
    }
    Scenario {
        base,
        events,
        world,
        indexed,
        log,
        dropped_rows,
        legacy,
    }
}

/// See the module doc: a folder with no row is not trashed, deleted or restored while it holds
/// anything. `rowless_while_inside` is every folder that had no row at some point it was in the
/// tree, not only the ones in it now: a folder can leave with its parent, be trashed while it is
/// outside, and be back when the parent is.
fn row_less_with_descendants(
    world: &World,
    rowless_while_inside: &HashSet<usize>,
    i: usize,
) -> bool {
    world.nodes[i].dir && rowless_while_inside.contains(&i) && !world.live_children(i).is_empty()
}

/// What a full walk with the scenario's rules lists at the end of the history.
fn walk_truth(scenario: &Scenario, options: &ScanOptions) -> Truth {
    let world = &scenario.world;
    (0..world.nodes.len())
        .filter(|&i| {
            i != ROOT_NODE && world.inside(i) && world.chain_live(i) && allowed(options, world, i)
        })
        .map(|i| {
            let node = &world.nodes[i];
            (
                world.rel_path(i),
                (node.dir, format!("vol~n{i}"), format!("h{}", node.sha)),
            )
        })
        .collect()
}

/// `None` when the map is what a walk lists, else what differs.
fn difference(
    truth: &Truth,
    remote: &HashMap<PathBuf, RemoteEntity>,
    scenario: &Scenario,
) -> Option<String> {
    let ignored = &scenario.dropped_rows;
    let mut diffs = Vec::new();
    for (path, (dir, id, sha)) in truth {
        if ignored.contains(path) {
            continue;
        }
        match remote.get(path) {
            None => diffs.push(format!("missing {}", path.display())),
            Some(entity) => {
                let is_dir = matches!(entity, RemoteEntity::Directory(_));
                // An untouched record under an id of an older index is the right node with the
                // wrong spelling of its id: a walk is what rewrites it, and nothing in a delta can.
                let untouched_legacy =
                    scenario.legacy.get(path).is_some_and(|(node, legacy_id)| {
                        entity.remote_id().as_deref() == Some(legacy_id.as_str())
                            && *id == format!("vol~n{node}")
                    });
                if is_dir != *dir
                    || (!untouched_legacy && entity.remote_id().as_deref() != Some(id.as_str()))
                {
                    diffs.push(format!(
                        "{}: wrong entity ({:?}, wanted {id})",
                        path.display(),
                        entity.remote_id()
                    ));
                } else if let RemoteEntity::File(file) = entity
                    && file.sha1_hash.as_deref() != Some(sha.as_str())
                {
                    diffs.push(format!(
                        "{}: stale content {:?} (wanted {sha})",
                        path.display(),
                        file.sha1_hash
                    ));
                }
            }
        }
    }
    for path in remote.keys() {
        if !truth.contains_key(path) && !ignored.contains(path) {
            diffs.push(format!("phantom {}", path.display()));
        }
    }
    diffs.sort();
    (!diffs.is_empty()).then(|| diffs.join("; "))
}

#[derive(Default, Debug)]
struct Tally {
    complete_correct: u64,
    fallback: u64,
    wrong: u64,
    /// The first few wrong cases, shortest history first, for the failure message.
    examples: Vec<(usize, String)>,
}

impl Tally {
    fn total(&self) -> u64 {
        self.complete_correct + self.fallback + self.wrong
    }

    fn note_wrong(&mut self, history: usize, text: String) {
        self.wrong += 1;
        self.examples.push((history, text));
        self.examples.sort_by_key(|(length, _)| *length);
        self.examples.truncate(3);
    }

    /// Fails on a wrong `Complete` map, and on a run that was too often a fallback to prove
    /// anything (a reconstruction that always walks is trivially never wrong).
    fn assert_sound(&self, label: &str, at_least_complete_percent: u64) {
        eprintln!(
            "[{label}] {} scenarios: {} complete and correct, {} fell back, {} complete and WRONG",
            self.total(),
            self.complete_correct,
            self.fallback,
            self.wrong
        );
        assert!(
            self.wrong == 0,
            "[{label}] {} of {} reconstructions were complete and wrong:\n{}",
            self.wrong,
            self.total(),
            self.examples
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(
            self.complete_correct * 100 >= self.total() * at_least_complete_percent,
            "[{label}] only {} of {} scenarios completed: the run does not exercise the \
             reconstruction",
            self.complete_correct,
            self.total()
        );
    }
}

fn resolver_for(scenario: &Scenario) -> ModelResolver {
    ModelResolver {
        world: scenario.world.clone(),
        indexed: scenario.indexed.clone(),
    }
}

fn describe_base(base: &HashMap<PathBuf, FileRecord>) -> String {
    let mut rows: Vec<String> = base
        .iter()
        .map(|(path, record)| {
            format!(
                "{}[{:?}|{:?}]",
                path.display(),
                record.entity_kind,
                record.proton_id
            )
        })
        .collect();
    rows.sort();
    rows.join(", ")
}

fn describe_events(events: &[RemoteChange]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "{:?}:{}@{:?}{}",
                event.kind,
                event.node_id,
                event.parent_id,
                if event.trashed { "T" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn run(shape: Shape, root_known: bool, count: u64) -> Tally {
    run_seeds(shape, root_known, first_seed()..first_seed() + count)
}

fn run_seeds(shape: Shape, root_known: bool, seeds: std::ops::Range<u64>) -> Tally {
    let options = shape.rules.options();
    let mut tally = Tally::default();
    for seed in seeds {
        let scenario = generate(seed, shape, &options);
        let truth = walk_truth(&scenario, &options);
        let outcome = reconstruct_remote(
            &scenario.base,
            &scenario.events,
            VOLUME,
            root_known.then_some(ROOT_UID),
            &options,
            &resolver_for(&scenario),
        );
        match outcome {
            Reconstruction::FallbackToSnapshot(_) => tally.fallback += 1,
            Reconstruction::Complete { remote, outside } => {
                match difference(&truth, &remote, &scenario) {
                    None => tally.complete_correct += 1,
                    Some(diff) => tally.note_wrong(
                        scenario.log.len(),
                        format!(
                            "seed {seed} rules={:?} top_ids={}\n  base: {}\n  history: {}\n  \
                             events: {}\n  outside={outside}  difference: {diff}",
                            shape.rules,
                            scenario.world.top_ids,
                            describe_base(&scenario.base),
                            scenario.log.join(" | "),
                            describe_events(&scenario.events),
                        ),
                    ),
                }
            }
        }
    }
    tally
}

fn shape(rules: Rules) -> Shape {
    Shape {
        rules,
        max_ops: 6,
        drop_folder_rows: false,
        drop_empty_folder_rows: false,
        retake_names: 0,
        quick_restore: 0,
        direct_delete: false,
        parentless: 0,
        legacy_ids: 0,
        trash_rowless_folders: false,
    }
}

#[test]
fn no_rules_a_complete_map_is_what_a_walk_lists() {
    run(shape(Rules::None), true, scenarios(DEFAULT_SCENARIOS * 2)).assert_sound("no rules", 55);
}

#[test]
fn no_rules_longer_histories() {
    let long = Shape {
        max_ops: 9,
        ..shape(Rules::None)
    };
    run(long, true, scenarios(DEFAULT_SCENARIOS)).assert_sound("no rules, long", 45);
}

#[test]
fn an_exclude_rule_changes_nothing() {
    run(shape(Rules::ExcludeB), true, scenarios(DEFAULT_SCENARIOS)).assert_sound("exclude", 50);
}

#[test]
fn include_rules_behave_as_if_the_root_were_never_learned() {
    for (rules, label) in [
        (Rules::IncludeAnyA, "include **/a"),
        (Rules::IncludeUnderB, "include b/**"),
        (Rules::IncludeUnderAOrAnyC, "include a/** and **/c"),
    ] {
        run(shape(rules), true, scenarios(DEFAULT_SCENARIOS)).assert_sound(label, 15);
    }
}

#[test]
fn an_index_with_folders_missing_its_rows_skips_nothing_it_cannot_justify() {
    let legacy = Shape {
        drop_folder_rows: true,
        trash_rowless_folders: true,
        ..shape(Rules::None)
    };
    run(legacy, true, scenarios(DEFAULT_SCENARIOS * 2)).assert_sound("folder rows missing", 10);
}

/// The adversarial orderings of round 2 of the review of #461: a node takes the name another just
/// left, a trash is undone at once, a node is deleted without ever being trashed.
#[test]
fn names_that_are_retaken_and_nodes_deleted_without_a_trash() {
    let adversarial = Shape {
        max_ops: 12,
        retake_names: 40,
        quick_restore: 25,
        direct_delete: true,
        ..shape(Rules::None)
    };
    run(adversarial, true, scenarios(DEFAULT_SCENARIOS)).assert_sound("retaken names", 30);
}

/// An event with no parent says nothing about where its node is.
#[test]
fn events_that_lose_their_parent_id_are_never_read_as_foreign() {
    let parentless = Shape {
        parentless: 10,
        max_ops: 9,
        ..shape(Rules::None)
    };
    run(parentless, true, scenarios(DEFAULT_SCENARIOS)).assert_sound("parentless events", 20);
}

/// A record whose id is a raw one, or another volume's, matches no event of this stream.
#[test]
fn records_under_the_ids_of_an_older_index_are_not_left_behind() {
    let older = Shape {
        legacy_ids: 25,
        max_ops: 9,
        ..shape(Rules::None)
    };
    run(older, true, scenarios(DEFAULT_SCENARIOS * 2)).assert_sound("older ids", 15);
}

/// A folder with no row and nothing beneath it is invisible to the index: a node the pair holds
/// can be moved into one, and must not be read as having left.
#[test]
fn a_node_moved_into_a_folder_with_no_row_is_not_read_as_gone() {
    let empty = Shape {
        drop_folder_rows: true,
        drop_empty_folder_rows: true,
        trash_rowless_folders: true,
        max_ops: 9,
        ..shape(Rules::None)
    };
    run(empty, true, scenarios(DEFAULT_SCENARIOS * 2)).assert_sound("empty folders, no rows", 10);
}

/// Seed 20700690 of the large run (`RECONSTRUCT_FUZZ_SCENARIOS=2000000 RECONSTRUCT_FUZZ_FROM=20000000`,
/// include `**/a`): a folder with no row, holding a record, was trashed while its parent was
/// temporarily outside the tree, and the parent came back. The generator judged "inside" at trash
/// time and let the history through; the folder is the same one with the same records wherever it is
/// when the event arrives, so the history is not generated.
#[test]
fn a_folder_trashed_while_its_parent_was_outside_the_tree_is_not_generated() {
    let tally = run_seeds(shape(Rules::IncludeAnyA), true, 20_700_690..20_700_691);
    assert_eq!(tally.wrong, 0, "{:?}", tally.examples);
}

/// Every adversarial shape together, with and without an exclude rule: the combinations are where
/// two masks that were each right alone stop being.
#[test]
fn every_adversarial_shape_at_once() {
    for (rules, label) in [
        (Rules::None, "everything at once"),
        (Rules::ExcludeB, "everything at once, exclude b/**"),
    ] {
        let all = Shape {
            max_ops: 12,
            drop_folder_rows: true,
            drop_empty_folder_rows: true,
            trash_rowless_folders: true,
            retake_names: 40,
            quick_restore: 25,
            direct_delete: true,
            parentless: 5,
            legacy_ids: 15,
            ..shape(rules)
        };
        run(all, true, scenarios(DEFAULT_SCENARIOS)).assert_sound(label, 3);
    }
}

/// The orderings of round 2 under include rules, which decide as before #456 and must stay as
/// sound as the code that did.
#[test]
fn the_adversarial_orderings_under_include_rules() {
    for (rules, label) in [
        (Rules::IncludeAnyA, "orderings, include **/a"),
        (Rules::IncludeUnderB, "orderings, include b/**"),
        (
            Rules::IncludeUnderAOrAnyC,
            "orderings, include a/** and **/c",
        ),
    ] {
        let orderings = Shape {
            max_ops: 12,
            retake_names: 40,
            quick_restore: 25,
            direct_delete: true,
            ..shape(rules)
        };
        run(orderings, true, scenarios(DEFAULT_SCENARIOS)).assert_sound(label, 5);
    }
}

/// A skip must also be right when the history is cut into two passes: the first delta's result is
/// committed as the next baseline (ids as listed) and the second continues from it, with the
/// listing always running ahead of both.
#[test]
fn two_passes_over_one_history_agree_with_a_walk() {
    let first = first_seed();
    let (tally, _) = two_passes(first..first + scenarios(DEFAULT_SCENARIOS));
    tally.assert_sound("two passes", 40);
}

/// Seed 91177341 of the large run (`RECONSTRUCT_FUZZ_SCENARIOS=2000000 RECONSTRUCT_FUZZ_FROM=90000000`):
/// the listing is ahead of the first part of the history, so it places a new folder at the path of a
/// tracked one, whose move out of the tree comes after the cut. The skip must take it (and only
/// because it is that case: the same history is wrong without the skip).
#[test]
fn a_move_out_of_a_node_the_first_pass_overwrote_is_not_compared() {
    let (tally, skipped) = two_passes(91_177_341..91_177_342);
    assert_eq!(tally.wrong, 0, "{:?}", tally.examples);
    assert_eq!(skipped, 1, "the seed no longer reaches the skip: {tally:?}");
}

/// Runs the two-pass check over a range of seeds; the second number is how many histories the skip
/// left out.
fn two_passes(seeds: std::ops::Range<u64>) -> (Tally, u64) {
    let options = Rules::None.options();
    let mut tally = Tally::default();
    let mut skipped = 0;
    for seed in seeds {
        let scenario = generate(seed, shape(Rules::None), &options);
        if scenario.events.len() < 2 {
            continue;
        }
        let truth = walk_truth(&scenario, &options);
        let cut = 1 + (seed as usize) % (scenario.events.len() - 1);
        let first = reconstruct_remote(
            &scenario.base,
            &scenario.events[..cut],
            VOLUME,
            Some(ROOT_UID),
            &options,
            &resolver_for(&scenario),
        );
        let Reconstruction::Complete { remote: middle, .. } = first else {
            tally.fallback += 1;
            continue;
        };
        let second_base = records_from_map(&middle);
        if ends_a_node_the_first_pass_overwrote(&scenario, cut, &second_base) {
            skipped += 1;
            continue;
        }
        let second_resolver = ModelResolver {
            world: scenario.world.clone(),
            indexed: second_base
                .iter()
                .filter_map(|(path, record)| Some((record.proton_id.clone()?, path.clone())))
                .collect(),
        };
        match reconstruct_remote(
            &second_base,
            &scenario.events[cut..],
            VOLUME,
            Some(ROOT_UID),
            &options,
            &second_resolver,
        ) {
            Reconstruction::FallbackToSnapshot(_) => tally.fallback += 1,
            Reconstruction::Complete { remote, outside } => {
                match difference(&truth, &remote, &scenario) {
                    None => tally.complete_correct += 1,
                    Some(diff) => tally.note_wrong(
                        scenario.log.len(),
                        format!(
                            "seed {seed} cut at {cut}/{}\n  base: {}\n  middle: {}\n  history: {}\n  \
                             events: {}\n  outside={outside}  difference: {diff}",
                            scenario.events.len(),
                            describe_base(&scenario.base),
                            describe_base(&second_base),
                            scenario.log.join(" | "),
                            describe_events(&scenario.events),
                        ),
                    ),
                }
            }
        }
    }
    (tally, skipped)
}

/// The limit the two-pass check leaves out (see the module doc): the first pass reads the events up
/// to the cut against a listing that is already ahead of them, so a node can be placed at a path
/// whose previous node is only trashed, deleted or moved out of the tree by an event after the cut.
/// That node's record is gone from the second pass's baseline, so the event reads as news about
/// someone else's node and what was beneath it stays. Older than #456 (the code before it
/// overwrote the record the same way) and healed by the next walk.
///
/// A move out is an update that is not a trash and names a parent that, by the history up to that
/// event, is not in the pair's tree. An update that names a parent inside it, or none, is not
/// skipped: a parentless event always walks.
fn ends_a_node_the_first_pass_overwrote(
    scenario: &Scenario,
    cut: usize,
    second_base: &HashMap<PathBuf, FileRecord>,
) -> bool {
    let ids_of = |records: &HashMap<PathBuf, FileRecord>| -> HashSet<String> {
        records
            .values()
            .filter_map(|record| record.proton_id.clone())
            .collect()
    };
    let (before, after) = (ids_of(&scenario.base), ids_of(second_base));
    let prefix = format!("{VOLUME}~");
    let node_of = |record: Option<&FileRecord>| -> Option<String> {
        let uid = record?.proton_id.as_deref()?;
        Some(uid.strip_prefix(&prefix)?.to_string())
    };
    let root = format!("n{ROOT_NODE}");

    // Which node each node sat in as the history reached each event: the baseline's folders first,
    // then every event's own parent. A node this cannot place is read as outside, which skips more
    // histories than the case needs and never fewer.
    let mut parent_of: HashMap<String, String> = HashMap::new();
    for (path, record) in &scenario.base {
        let Some(node) = node_of(Some(record)) else {
            continue;
        };
        let parent = match path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            None => root.clone(),
            Some(parent) => match node_of(scenario.base.get(parent)) {
                Some(parent) => parent,
                None => continue,
            },
        };
        parent_of.insert(node, parent);
    }
    fn inside<'a>(parent_of: &'a HashMap<String, String>, mut node: &'a str, root: &str) -> bool {
        for _ in 0..=parent_of.len() {
            if node == root {
                return true;
            }
            match parent_of.get(node) {
                Some(parent) => node = parent,
                None => return false,
            }
        }
        false
    }

    for (index, event) in scenario.events.iter().enumerate() {
        let uid = node_uid(VOLUME, &event.node_id);
        if index >= cut && before.contains(&uid) && !after.contains(&uid) {
            let removal = matches!(event.kind, RemoteChangeKind::Deleted)
                || (matches!(event.kind, RemoteChangeKind::Updated) && event.trashed);
            let moves_out = matches!(event.kind, RemoteChangeKind::Updated)
                && !event.trashed
                && event
                    .parent_id
                    .as_deref()
                    .is_some_and(|parent| !inside(&parent_of, parent, &root));
            if removal || moves_out {
                return true;
            }
        }
        if let Some(parent) = &event.parent_id {
            parent_of.insert(event.node_id.clone(), parent.clone());
        }
    }
    false
}

/// The baseline a second pass starts from: the first pass's map, committed with the ids it listed.
fn records_from_map(remote: &HashMap<PathBuf, RemoteEntity>) -> HashMap<PathBuf, FileRecord> {
    remote
        .iter()
        .map(|(path, entity)| {
            let proton_id = entity.remote_id().filter(|id| !id.is_empty());
            let record = match entity {
                RemoteEntity::Directory(_) => FileRecord {
                    file_path: path.clone(),
                    entity_kind: EntityKind::Directory,
                    file_size: 0,
                    mtime: 0,
                    sha1_hash: None,
                    proton_id,
                    sync_status: SyncStatus::Synced,
                },
                RemoteEntity::File(file) => FileRecord {
                    file_path: path.clone(),
                    entity_kind: EntityKind::File,
                    file_size: 1,
                    mtime: 0,
                    sha1_hash: file.sha1_hash.clone(),
                    proton_id,
                    sync_status: SyncStatus::Synced,
                },
            };
            (path.clone(), record)
        })
        .collect()
}
