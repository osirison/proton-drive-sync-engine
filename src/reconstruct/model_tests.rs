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
//! configuration and takes about a second; the large run is the same code with more seeds:
//!
//! ```bash
//! RECONSTRUCT_FUZZ_SCENARIOS=300000 cargo test --lib reconstruct::model_tests -- --nocapture
//! ```
//!
//! ## What the model deliberately leaves out
//!
//! Each of these is a limit of volume events that is older than #456 and equally present when the
//! pair does not know its root (the "control" runs the review used), so it is not generated rather
//! than reported:
//!
//! * a directory **with descendants** is never renamed or moved *within* the tree: events are per
//!   link, so nothing re-keys the descendants already in the map;
//! * a folder **that has no index row** is never trashed, deleted, restored or moved into the tree
//!   while it has descendants: the event is a single one for the folder's own link and the pair,
//!   which does not track the folder, cannot tell which of its records are under it. (Include
//!   rules and indexes written before folders were rows both produce such folders.) A pair with
//!   include rules does not use the root uid at all, so these cases are decided exactly as before
//!   #456 and by the same code.

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
}

#[derive(Clone, Copy)]
struct Shape {
    rules: Rules,
    max_ops: usize,
    /// Some folders of the baseline have no row (an index written before folders were rows).
    drop_folder_rows: bool,
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
        base.insert(world.rel_path(i), record_of(&world, i, true));
        in_base.insert(i);
    }
    if shape.drop_folder_rows {
        // Only a folder with a record beneath it: that record is the evidence the index gives that
        // the folder exists. A folder with no row and nothing beneath it is invisible to the index
        // (it arises when rules are loosened, and a `resync` heals it); see the ADR.
        let mut folders: Vec<usize> = in_base
            .iter()
            .copied()
            .filter(|&i| world.nodes[i].dir)
            .collect();
        folders.sort_unstable();
        for i in folders {
            let path = world.rel_path(i);
            let holds_a_record = base
                .keys()
                .any(|other| other != &path && other.starts_with(&path));
            if holds_a_record && rng.chance(40) {
                base.remove(&path);
                in_base.remove(&i);
                rowless.insert(i);
                dropped_rows.insert(path);
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
    let mut sha_counter = 10;
    let has_daemon_made_record = |base: &HashMap<PathBuf, FileRecord>, world: &World, i: usize| {
        world.inside(i)
            && base
                .values()
                .any(|r| r.proton_id.is_none() && world.rel_path(i) == r.file_path)
    };

    for _ in 0..(1 + rng.below(shape.max_ops)) {
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
                if world
                    .live_children(parent)
                    .iter()
                    .any(|&child| world.nodes[child].name == name)
                    || has_daemon_made_record(&base, &world, i)
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
                {
                    continue;
                }
                // See the module doc: a directory with descendants is not moved within the tree,
                // and a folder without a row is not moved into it while it holds anything.
                if world.nodes[i].dir && !world.live_children(i).is_empty() {
                    let lands_as_an_entity = now_inside
                        && options.allows_relative_directory(&world.rel_path(target).join(&name));
                    if (was_inside && now_inside) || (now_inside && !lands_as_an_entity) {
                        continue;
                    }
                }
                world.nodes[i].parent = Some(target);
                structural_inside |= was_inside || now_inside;
                log.push(format!(
                    "move {} node {i} ({name}) {} -> {} under node {target}",
                    if world.nodes[i].dir { "dir" } else { "file" },
                    if was_inside { "inside" } else { "outside" },
                    if now_inside { "inside" } else { "outside" },
                ));
                events.push(world.event(RemoteChangeKind::Updated, i, false, events.len()));
            }
            // trash
            8 => {
                if live_nodes.is_empty() {
                    continue;
                }
                let i = live_nodes[rng.below(live_nodes.len())];
                if row_less_with_descendants(&world, &rowless, options, i) {
                    continue;
                }
                structural_inside |= world.inside(i);
                world.nodes[i].trashed = true;
                log.push(format!("trash node {i}"));
                events.push(world.event(RemoteChangeKind::Updated, i, true, events.len()));
            }
            // restore a trashed node, or delete it for good
            _ => {
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
                        || row_less_with_descendants(&world, &rowless, options, i)
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
                }
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
    }
}

/// See the module doc: a folder with no row is not trashed while it holds anything.
fn row_less_with_descendants(
    world: &World,
    rowless: &HashSet<usize>,
    options: &ScanOptions,
    i: usize,
) -> bool {
    world.nodes[i].dir
        && world.inside(i)
        && (rowless.contains(&i) || !allowed(options, world, i))
        && !world.live_children(i).is_empty()
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
    ignored: &HashSet<PathBuf>,
) -> Option<String> {
    let mut diffs = Vec::new();
    for (path, (dir, id, sha)) in truth {
        if ignored.contains(path) {
            continue;
        }
        match remote.get(path) {
            None => diffs.push(format!("missing {}", path.display())),
            Some(entity) => {
                let is_dir = matches!(entity, RemoteEntity::Directory(_));
                if is_dir != *dir || entity.remote_id().as_deref() != Some(id.as_str()) {
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

fn run(shape: Shape, root_known: bool, seeds: u64) -> Tally {
    let options = shape.rules.options();
    let mut tally = Tally::default();
    for seed in 0..seeds {
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
            Reconstruction::Complete { remote, outside } => match difference(
                &truth,
                &remote,
                &scenario.dropped_rows,
            ) {
                None => tally.complete_correct += 1,
                Some(diff) => tally.note_wrong(
                    scenario.log.len(),
                    format!(
                        "seed {seed} rules={:?} top_ids={}\n  history: {}\n  outside={outside}  \
                         difference: {diff}",
                        shape.rules,
                        scenario.world.top_ids,
                        scenario.log.join(" | ")
                    ),
                ),
            },
        }
    }
    tally
}

fn shape(rules: Rules) -> Shape {
    Shape {
        rules,
        max_ops: 6,
        drop_folder_rows: false,
    }
}

#[test]
fn no_rules_a_complete_map_is_what_a_walk_lists() {
    run(shape(Rules::None), true, scenarios(DEFAULT_SCENARIOS * 2)).assert_sound("no rules", 70);
}

#[test]
fn no_rules_longer_histories() {
    let long = Shape {
        max_ops: 9,
        ..shape(Rules::None)
    };
    run(long, true, scenarios(DEFAULT_SCENARIOS)).assert_sound("no rules, long", 60);
}

#[test]
fn an_exclude_rule_changes_nothing() {
    run(shape(Rules::ExcludeB), true, scenarios(DEFAULT_SCENARIOS)).assert_sound("exclude", 70);
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
        ..shape(Rules::None)
    };
    run(legacy, true, scenarios(DEFAULT_SCENARIOS * 2)).assert_sound("folder rows missing", 10);
}

/// A skip must also be right when the history is cut into two passes: the first delta's result is
/// committed as the next baseline (ids as listed) and the second continues from it, with the
/// listing always running ahead of both.
#[test]
fn two_passes_over_one_history_agree_with_a_walk() {
    let options = Rules::None.options();
    let mut tally = Tally::default();
    for seed in 0..scenarios(DEFAULT_SCENARIOS) {
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
                match difference(&truth, &remote, &scenario.dropped_rows) {
                    None => tally.complete_correct += 1,
                    Some(diff) => tally.note_wrong(
                        scenario.log.len(),
                        format!(
                            "seed {seed} cut at {cut}/{}\n  history: {}\n  outside={outside}  \
                         difference: {diff}",
                            scenario.events.len(),
                            scenario.log.join(" | ")
                        ),
                    ),
                }
            }
        }
    }
    tally.assert_sound("two passes", 40);
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
