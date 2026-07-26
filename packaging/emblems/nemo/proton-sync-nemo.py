# Proton Drive Sync — Nemo file-manager emblems (S10, #91).
#
# Nemo (Cinnamon) port of the Nautilus emblem provider. Nemo's python API mirrors Nautilus's, so the
# index logic is identical; it is intentionally duplicated here (rather than a shared import) so each
# extension file loads standalone from its own file-manager's extension dir with no sys.path setup.
# See packaging/emblems/nautilus/proton-sync-nautilus.py for the design/correctness notes.

import os
import sqlite3
import threading

import gi

gi.require_version("Nemo", "3.0")
from gi.repository import GObject, Nemo  # noqa: E402

STATE_DIR = ".sync"
INDEX_DB = "sync_index.db"

EMBLEM_FOR = {
    "synced": "emblem-proton-sync-synced",
    "modified": "emblem-proton-sync-syncing",
    "conflict": "emblem-proton-sync-conflict",
}


class _Index:
    def __init__(self):
        self._lock = threading.Lock()
        self._conns = {}
        self._root_of_dir = {}

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
            if parent == cur:
                break
            cur = parent
        # Only memoize POSITIVE results — caching None would hide a folder that becomes a sync root
        # later (daemon starts) until Nemo restarts. Misses re-walk (cheap).
        if result is not None:
            for d in chain:
                self._root_of_dir[d] = result
        return result

    def _conn(self, db_path):
        conn = self._conns.get(db_path)
        if conn is None:
            conn = sqlite3.connect("file:" + db_path + "?mode=ro", uri=True, check_same_thread=False, timeout=3.0)
            conn.execute("PRAGMA busy_timeout = 3000")
            self._conns[db_path] = conn
        return conn

    def status_for(self, abs_path):
        directory = os.path.dirname(abs_path)
        with self._lock:
            found = self._find_root(directory)
            if not found:
                return None
            root, db_path = found
            rel = os.path.relpath(abs_path, root)
            if rel.startswith(".."):
                return None
            try:
                conn = self._conn(db_path)
                row = conn.execute(
                    "SELECT sync_status FROM file_index WHERE file_path = ? OR file_path = ? LIMIT 1",
                    (rel, os.fsencode(rel)),
                ).fetchone()
            except sqlite3.Error:
                self._conns.pop(db_path, None)
                return None
            return row[0] if row else None


_INDEX = _Index()


class ProtonSyncEmblems(GObject.GObject, Nemo.InfoProvider):
    def update_file_info(self, file):
        if file.get_uri_scheme() != "file":
            return Nemo.OperationResult.COMPLETE
        location = file.get_location()
        path = location.get_path() if location is not None else None
        if not path:
            return Nemo.OperationResult.COMPLETE
        status = _INDEX.status_for(path)
        emblem = EMBLEM_FOR.get(status)
        if emblem:
            file.add_emblem(emblem)
        return Nemo.OperationResult.COMPLETE
