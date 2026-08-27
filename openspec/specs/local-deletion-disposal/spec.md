## Purpose

Defines how the engine disposes of a local file or folder when it mirrors a deletion that happened
on Proton Drive — whether the entity is moved to the desktop trash or removed outright, how the user
chooses between the two, and which warnings each choice carries.

## Requirements

### Requirement: Local deletions are recoverable by default

A local deletion mirrored from the remote SHALL move the entity to the desktop trash rather than
remove it, unless the user has chosen permanent deletion for that folder pair. A configuration that
says nothing about disposal SHALL be read as trash.

#### Scenario: A remote deletion reaches a configuration that says nothing

- **WHEN** a file synced to disk is deleted on Proton Drive, the deletion is approved or ungated, and
  the configuration names no disposal mode
- **THEN** the file is no longer at its path in the sync folder
- **AND** the file is present in the desktop trash with its contents intact

#### Scenario: A folder is trashed whole

- **WHEN** a synced folder holding files and subfolders is deleted on Proton Drive and the deletion
  applies in trash mode
- **THEN** the whole folder is moved to the trash as one entity, with its contents still inside it
- **AND** no part of the folder remains in the sync folder

### Requirement: A trashed entity is restorable from the desktop

A trashed entity SHALL be placed where the desktop environment's own trash implementation expects
it, with the metadata that implementation needs to restore it, so that a user can find and restore
it through their file manager without knowing anything about this engine.

#### Scenario: Restoring through the file manager

- **WHEN** an entity has been trashed by the engine
- **AND** the user opens their desktop file manager's Trash and restores it
- **THEN** the entity reappears at the path it was deleted from

#### Scenario: The trash records where the entity came from

- **WHEN** an entity has been trashed by the engine
- **THEN** the trash records that entity's original absolute path and the time it was trashed

### Requirement: Permanent deletion remains available and unchanged

A folder pair configured for permanent local deletion SHALL remove the entity from disk exactly as
before this capability existed: no trash copy, no metadata, and the disk space released immediately.

#### Scenario: Permanent mode removes the file outright

- **WHEN** the configuration selects permanent local deletion
- **AND** a synced file is deleted on Proton Drive and the deletion applies
- **THEN** the file is gone from its path
- **AND** nothing corresponding to it appears in the desktop trash

### Requirement: A failed trash move is a failed item, never a silent removal

When an entity cannot be moved to the trash, the engine SHALL NOT remove it from disk instead. The
action SHALL be reported as a failed item, the rest of the pass SHALL continue, and the deletion
SHALL be attempted again on a later pass.

#### Scenario: The trash cannot accept the entity

- **WHEN** a local deletion applies in trash mode and the move to the trash fails for any reason
- **THEN** the entity is still at its original path with its contents intact
- **AND** the pass reports it as a failed item naming that path
- **AND** the remaining actions in the pass still execute
- **AND** the entity's baseline record is not purged, so the deletion is planned again next pass

#### Scenario: The trash is unavailable for the whole pass

- **WHEN** every local deletion in a pass fails to reach the trash
- **THEN** no entity that was to be deleted has been removed from disk

### Requirement: The disposal that will be used is reported with each pending deletion

Each deletion waiting for a decision SHALL be reported together with the disposal the engine will
actually apply to it, so that a client can describe the consequence without reading configuration
files that the running daemon may not have loaded.

#### Scenario: A pending local deletion in trash mode

- **WHEN** a client asks the daemon for the deletions waiting on a decision
- **AND** the daemon is running in trash mode
- **THEN** each pending local deletion is reported as recoverable

#### Scenario: A client older than this capability

- **WHEN** a reply carries no disposal for a pending deletion, because the daemon predates this
  capability or the field is otherwise absent
- **THEN** the deletion is treated as permanent, which is the more cautious of the two readings

### Requirement: Warnings follow the consequence, not the direction

The warnings, confirmation friction and interruptions attached to a deletion SHALL be determined by
whether that deletion is actually recoverable, not by which side it applies to. A deletion that can
be undone SHALL NOT be presented as irreversible.

#### Scenario: A trashed local deletion is presented as recoverable

- **WHEN** the user views a local deletion waiting on a decision while trash mode is on
- **THEN** it is presented as recoverable, alongside deletions that go to Proton Drive's Trash
- **AND** it can be approved in a single action, with no word to type
- **AND** no copy claims the file is removed for good or is unrecoverable

#### Scenario: A permanent local deletion keeps its warnings

- **WHEN** the user views a local deletion waiting on a decision while permanent mode is on
- **THEN** it is presented as permanent and separated from the recoverable ones
- **AND** approving it requires the explicit typed confirmation
- **AND** the copy states that the entity is removed from this computer for good

#### Scenario: Recoverable deletions do not interrupt

- **WHEN** local deletions are waiting on a decision in trash mode and nothing else is notable
- **THEN** no interrupting notification is raised about them

#### Scenario: Permanent deletions still interrupt

- **WHEN** local deletions are waiting on a decision in permanent mode
- **THEN** an interrupting notification is raised naming what would be lost

### Requirement: The disposal choice is a folder-pair setting the user can change

The disposal mode SHALL be readable and writable both as a configuration-file key and from the
application's deletion settings, and SHALL be scoped to a folder pair rather than to the whole
daemon. An invalid value SHALL be refused when the configuration is read, naming the key and the
values it accepts.

#### Scenario: Choosing permanent deletion in settings

- **WHEN** the user selects permanent local deletion in the deletion settings and saves
- **THEN** the configuration file records that choice
- **AND** reopening the settings shows permanent deletion selected
- **AND** the daemon applies it once restarted

#### Scenario: An unrecognised value is refused

- **WHEN** a configuration file sets the disposal mode to a value that is neither of the two accepted
  ones
- **THEN** reading that configuration fails with an error naming the key and listing the accepted
  values
- **AND** the daemon does not start with an assumed default

### Requirement: Approval gating is not changed by the disposal mode

Which deletions wait for a person SHALL continue to be decided by the existing deletion-approval
setting, keyed on the direction of the deletion. Turning trash mode on or off SHALL NOT change
whether a deletion waits.

#### Scenario: An existing configuration keeps its gating

- **WHEN** a configuration gates local deletions for approval and does not name a disposal mode
- **THEN** local deletions still wait for approval, and are trashed rather than removed once approved

#### Scenario: Switching disposal mode leaves gating alone

- **WHEN** the user switches between trash and permanent local deletion
- **THEN** the set of deletions that wait for a decision is unchanged

### Requirement: The trash is never synced

Anything the engine places in the trash SHALL NOT be treated as content of the sync folder. If the
trash location for an entity falls inside a folder pair's local root, the engine SHALL ignore it when
scanning, planning and watching, so that a trashed entity is never uploaded back to Proton Drive.

#### Scenario: A trash directory inside the sync root

- **WHEN** the sync folder is on a filesystem whose trash location lies inside that folder
- **AND** an entity is trashed there
- **THEN** the next pass plans no upload for anything under that trash location
- **AND** the trashed entity does not reappear on Proton Drive
