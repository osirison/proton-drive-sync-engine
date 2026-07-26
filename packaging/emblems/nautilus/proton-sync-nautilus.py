# Proton Drive Sync — Nautilus file-manager emblems (S10, #91).
#
# Shows a per-file emblem (synced / syncing / conflict) in Nautilus by reading the sync engine's
# SQLite index READ-ONLY. It is an *enhancement*, never the only signal of sync state.
#
# Correctness notes (these matter — get them wrong and emblems silently don't show):
#  * Index discovery: for a file, walk UP its ancestors to the sync root that contains
#    `.sync/sync_index.db`. We do NOT hardcode the GUI's config path — the daemon may run with a
#    different --config/--db-path, and there may be more than one sync root.
#  * Key encoding: `file_index.file_path` is the path relative to the root, stored byte-exact — TEXT
#    for valid UTF-8, BLOB otherwise. For a UTF-8 name we match both (str AND raw bytes). A non-UTF-8
#    (surrogateescaped) name canNOT be bound as a TEXT param — sqlite3 would raise UnicodeEncodeError
#    (not a sqlite3.Error, so uncaught → crashes the file manager) — so we match it by its BLOB key
#    alone.
#  * Read-only + busy_timeout: the daemon's index runs WITHOUT WAL, so a naive reader races its write
#    transactions. We open `mode=ro` and set a busy timeout instead of failing on SQLITE_BUSY.
#  * Nautilus 4 API (Fedora/GNOME ship 4.x): `Nautilus.InfoProvider.update_file_info` returns
#    OperationResult.COMPLETE; emblems are added via `FileInfo.add_emblem`.

import os
import sqlite3
import threading

import gi

gi.require_version("Nautilus", "4.0")
from gi.repository import GObject, Nautilus  # noqa: E402

STATE_DIR = ".sync"
INDEX_DB = "sync_index.db"

# Only the three states the index actually stores. "excluded"/"paused" need selective-sync globs /
# live daemon state that aren't in the index, so they're intentionally out of scope here (v1).
EMBLEM_FOR = {
    "synced": "emblem-proton-sync-synced",
    "modified": "emblem-proton-sync-syncing",
    "conflict": "emblem-proton-sync-conflict",
}


class _Index:
    """Locates sync roots by walking up, and caches one read-only connection per index DB."""

    def __init__(self):
        self._lock = threading.Lock()
        self._conns = {}  # db_path -> sqlite3.Connection (read-only)
        self._root_of_dir = {}  # dir -> (root, db_path) | None  (memoized ancestor walk)

    def _find_root(self, directory):
        if directory in self._root_of_dir:
            return self._root_of_dir[directory]
        cur = directory
        chain = []
        result = None
        while True:
            chain.append(cur)
            db = os.path.join(cur, STATE_DIR, INDEX_DB)
            if os.path.isfile(db):
                result = (cur, db)
                break
            parent = os.path.dirname(cur)
            if parent == cur:  # reached filesystem root
                break
            cur = parent
        # Only memoize POSITIVE results: a directory that isn't a root yet may become one later (the
        # daemon starts / `.sync/sync_index.db` appears), so caching `None` would hide new roots
        # until the file manager restarts. Misses re-walk (cheap: a few stats up the tree).
        if result is not None:
            for d in chain:
                self._root_of_dir[d] = result
        return result

    def _conn(self, db_path):
        conn = self._conns.get(db_path)
        if conn is None:
            uri = "file:" + db_path + "?mode=ro"
            conn = sqlite3.connect(uri, uri=True, check_same_thread=False, timeout=3.0)
            conn.execute("PRAGMA busy_timeout = 3000")
            self._conns[db_path] = conn
        return conn

    def status_for(self, abs_path):
        """Return 'synced' | 'modified' | 'conflict' for abs_path, or None if untracked/unknown."""
        directory = os.path.dirname(abs_path)
        with self._lock:
            found = self._find_root(directory)
            if not found:
                return None
            root, db_path = found
            rel = os.path.relpath(abs_path, root)
            if rel.startswith(".."):
                return None
            row = self._lookup(db_path, rel)
            return row[0] if row else None

    def _lookup(self, db_path, rel):
        """Query the index for `rel`, matching both TEXT and BLOB key encodings — see header."""
        key_blob = os.fsencode(rel)
        try:
            rel.encode("utf-8")
        except UnicodeEncodeError:
            # Non-UTF-8 (surrogateescaped) name: it can only be a BLOB key, and binding it as TEXT
            # would raise UnicodeEncodeError (not a sqlite3.Error) and crash the file manager.
            text_key = None
        else:
            text_key = rel
        try:
            conn = self._conn(db_path)
            if text_key is not None:
                return conn.execute(
                    "SELECT sync_status FROM file_index WHERE file_path = ? OR file_path = ? LIMIT 1",
                    (text_key, key_blob),
                ).fetchone()
            return conn.execute(
                "SELECT sync_status FROM file_index WHERE file_path = ? LIMIT 1",
                (key_blob,),
            ).fetchone()
        except sqlite3.Error:
            # A transient read error (locked/mid-migration) → no emblem this pass, not a crash.
            self._conns.pop(db_path, None)
            return None


_INDEX = _Index()


class ProtonSyncEmblems(GObject.GObject, Nautilus.InfoProvider):
    def update_file_info(self, file):
        if file.get_uri_scheme() != "file":
            return Nautilus.OperationResult.COMPLETE
        location = file.get_location()
        path = location.get_path() if location is not None else None
        if not path:
            return Nautilus.OperationResult.COMPLETE
        status = _INDEX.status_for(path)
        emblem = EMBLEM_FOR.get(status)
        if emblem:
            file.add_emblem(emblem)
        return Nautilus.OperationResult.COMPLETE
