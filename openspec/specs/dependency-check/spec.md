## Purpose

Defines the check that runs before the first sync. It covers what is checked, how each result is
stated, which fix and links each problem carries, and what `proton-sync doctor`, the desktop app's
first run, `setup.sh` and the daemon's start-up do with the answer.

## Requirements

### Requirement: A fixed set of checks, each required or recommended

The dependency check SHALL report on these checks, each with a stable id and a fixed severity:
`proton-drive`, `signed-in` and `config` are required; `secret-tool`, `curl`, `session-bus` and
`service` are recommended. A required check is one without which no sync can work. A recommended
check is one without which sync still works but more slowly, or without one of its features.

#### Scenario: The report names every check

- **WHEN** `proton-sync doctor --json` runs
- **THEN** its output lists exactly the seven check ids above
- **AND** each one is marked required or recommended as stated above

#### Scenario: A recommended problem says what still works

- **WHEN** `curl` or `secret-tool` is not installed
- **THEN** the problem is reported as recommended
- **AND** its text says that sync still works but every pass walks the whole Proton Drive folder
  instead of following changes

### Requirement: Three results, never two

Every check SHALL end in exactly one of three results: ok, a problem (the thing is missing or does
not work), or could not check. A check that could not be checked SHALL NOT be reported as ok and
SHALL NOT be reported as missing, and its text SHALL say why it could not be checked.

#### Scenario: No session bus

- **WHEN** the check runs in a shell with no session bus address set
- **THEN** `session-bus` is reported as a problem
- **AND** `signed-in` is reported as could not check, naming the missing session bus as the reason
- **AND** the keyring is not queried

#### Scenario: Credentials kept outside the keyring

- **WHEN** `PROTON_DRIVE_CREDENTIALS_STORE` is set to `pass` or `unsafe_file`
- **THEN** `signed-in` is reported as could not check, not as "not signed in"
- **AND** its text says that the daemon's change detection reads the sign-in only from the default
  keychain store

### Requirement: The CLI is judged by running its version command

The `proton-drive` check SHALL run the configured CLI with the single argument `version`, with no
input and within a time limit. It SHALL judge the result by how the process ended, not by whether
it started. "Not found", "not executable", "crashed" and "exited with an error" SHALL each be
reported as a distinct problem with its own fix.

#### Scenario: The CLI is not installed

- **WHEN** no `proton-drive` executable is found at the configured path or on `PATH`
- **THEN** `proton-drive` is reported as missing
- **AND** the fix says to install it from `https://proton.me/drive/download` and then run
  `proton-drive auth login`

#### Scenario: The CLI starts but exits with an error

- **WHEN** `proton-drive version` exits with a non-zero status
- **THEN** `proton-drive` is reported as a problem that says it was found but did not run
  correctly, with the exit status
- **AND** it is not reported as missing

#### Scenario: The CLI crashes on a CPU without AVX2

- **WHEN** `proton-drive version` is killed by an illegal-instruction signal
- **THEN** `proton-drive` is reported as a problem
- **AND** the fix names the `linux/x64-baseline` build on Proton's download page

#### Scenario: The CLI does not answer

- **WHEN** `proton-drive version` is still running when the time limit runs out
- **THEN** the process is stopped
- **AND** `proton-drive` is reported as could not check

#### Scenario: The CLI works

- **WHEN** `proton-drive version` exits with status 0
- **THEN** `proton-drive` is reported as ok, with the version it printed and the full path of the
  executable that ran

#### Scenario: The CLI path is relative

- **WHEN** the configured or given `proton_cli` contains a `/` and is not absolute
- **THEN** `proton-drive` is reported as a problem that says the service resolves a relative path
  against its own working directory, and the fix says to set an absolute path
- **AND** nothing is run, and the answer does not depend on the directory `doctor` runs in
- **AND** a bare name with no `/` is still searched on `PATH`

#### Scenario: The CLI path is a directory

- **WHEN** `proton_cli` names a directory
- **THEN** `proton-drive` is reported as a problem that says it is a directory and not the
  proton-drive program, and the fix says to point `proton_cli` at the program file
- **AND** it is not reported as "not executable"

#### Scenario: The CLI prints control characters

- **WHEN** the CLI's first output line holds an escape sequence or another control character
- **THEN** each control character is replaced where the output is captured, so no report line, log
  line or app screen carries it

### Requirement: Only the version command is run against the CLI

The dependency check SHALL NOT run any `proton-drive` command other than `version`. It SHALL NOT
list, upload, download or sign in, so running it beside a live daemon starts no second client of
the CLI's shared state.

#### Scenario: The check runs while the daemon is syncing

- **WHEN** `proton-sync doctor` runs while `proton-syncd` is running
- **THEN** the only `proton-drive` process it starts is `proton-drive version`

### Requirement: Sign-in is checked without reading the secret

The `signed-in` check SHALL ask the desktop keyring whether the CLI's session entry exists and
SHALL judge only by the answer's status. The session secret SHALL NOT be read into the program,
printed, logged or kept. An ok result SHALL say that a sign-in is stored, not that Proton accepts
it.

#### Scenario: A sign-in is stored

- **WHEN** the keyring holds an entry for the CLI's session
- **THEN** `signed-in` is reported as ok, worded as "A sign-in is stored"
- **AND** no part of the stored secret appears in the output, in `--json`, or in any log

#### Scenario: No sign-in is stored

- **WHEN** the keyring answers that it holds no entry for the CLI's session
- **THEN** `signed-in` is reported as a problem
- **AND** the fix says to run `proton-drive auth login` and then check again

#### Scenario: The keyring answers with an error

- **WHEN** the keyring lookup fails for a reason other than "no such entry" (the keyring is locked
  and the unlock prompt was dismissed, or no Secret Service is running)
- **THEN** `signed-in` is reported as could not check, with the reason
- **AND** it is not reported as "not signed in"

#### Scenario: secret-tool is not installed

- **WHEN** `secret-tool` is not on `PATH`
- **THEN** `secret-tool` is reported as a problem
- **AND** `signed-in` is reported as could not check

### Requirement: The config file is read, not assumed

The `config` check SHALL read the config file named by `--config`, or else the default config file
that the desktop app and the packaged service use. It SHALL report a file that exists but cannot be
read or does not validate as a problem, quoting the error. No file at the default location SHALL be
reported as ok and worded as "No config file yet (the default is <path>)".

#### Scenario: The config file does not validate

- **WHEN** the config file has a syntax error or a value the daemon refuses
- **THEN** `config` is reported as a problem, quoting the error the daemon would give
- **AND** the `proton-drive` check still runs, against `proton-drive` on `PATH`, and its text says
  that is what it checked

#### Scenario: The config names the CLI

- **WHEN** the config file sets `proton_cli` to an absolute path
- **THEN** the `proton-drive` check runs that path, not `proton-drive` on `PATH`

#### Scenario: A named config file is missing

- **WHEN** `--config` names a file that does not exist
- **THEN** `config` is reported as a problem naming that path

#### Scenario: There is no home directory

- **WHEN** neither `XDG_CONFIG_HOME` nor `HOME` is set to an absolute path and no `--config` is given
- **THEN** no config file is read, including any file in the shared temporary directory
- **AND** `config` is reported as could not check, saying that no home directory means the default
  file cannot be located
- **AND** the `proton-drive` check runs `proton-drive` from `PATH`, never a `proton_cli` taken from a
  file in a directory other users can write

### Requirement: Every problem carries its fix and links, and CLI links are Proton's

Every problem and every could-not-check result SHALL carry a one-line fix and a link to its own
section of the troubleshooting page on the docs site. A result about the `proton-drive` CLI SHALL
also link Proton's own page and SHALL NOT name or link any other build of the CLI. Every link SHALL
be printed as a full `https://` URL with no punctuation attached to it.

#### Scenario: The download link is printed exactly

- **WHEN** `proton-drive` is reported as missing
- **THEN** the human output contains `https://proton.me/drive/download` followed by whitespace or the
  end of a line

#### Scenario: Only Proton's pages are linked for the CLI

- **WHEN** the output of any check about the CLI is read
- **THEN** every link in it is one of `https://proton.me/drive/download`,
  `https://proton.me/download/drive/cli/index.html`, `https://proton.me/support/drive-cli`,
  `https://github.com/ProtonDriveApps/sdk/blob/main/cli/README.md`, or a page on this project's
  docs site
- **AND** no community repackage of the CLI is named

#### Scenario: The sign-in command is Proton's

- **WHEN** any result gives the command to sign in
- **THEN** the command is `proton-drive auth login`

#### Scenario: Sentences start with a capital

- **WHEN** any statement or fix is read
- **THEN** it starts with a capital letter, unless it starts with a program name or a path

### Requirement: Every docs link the check emits resolves

Every docs link the check emits SHALL point to a heading that exists in the docs site's source,
under the site's published base path. A build of the repository SHALL fail when an emitted link has
no matching heading.

#### Scenario: A heading is renamed

- **WHEN** a troubleshooting heading that a check links to is renamed and the link is not updated
- **THEN** `cargo test` fails, naming the link and the page

#### Scenario: The base path changes

- **WHEN** the docs site's default base path in its Astro config no longer matches the prefix of
  the links the check emits
- **THEN** `cargo test` fails

### Requirement: proton-sync doctor runs without the daemon

`proton-sync doctor` SHALL run the check and print the report without connecting to the daemon's
control socket. It SHALL NOT start the daemon, and SHALL NOT create the control socket or any
directory for it. It SHALL accept `proton-sync [--config FILE] [--json] doctor [--proton-cli PATH]`
and SHALL refuse `--socket-path`, `--pair` and `--all-pairs`.

#### Scenario: A clean system without the CLI

- **WHEN** `proton-sync doctor` runs where the package is installed, no `proton-drive` is on `PATH`,
  no config file exists, no session bus is set and no daemon is running
- **THEN** its output names `proton-drive` as missing and contains `https://proton.me/drive/download`
- **AND** it exits with status 1
- **AND** no `proton-syncd` process is started, and no control socket or socket directory is created

#### Scenario: Nothing is created on any path

- **WHEN** `proton-sync doctor` runs with `HOME` and `XDG_STATE_HOME` unset
- **THEN** no directory or file is created, including the per-user directory under the shared
  temporary directory where a daemon's lock would be
- **AND** a daemon lock that is not there is read as "no daemon is running"

#### Scenario: A pair selector is refused

- **WHEN** `proton-sync --pair photos doctor` runs
- **THEN** it exits with status 2 and says that `doctor` does not address a folder pair

#### Scenario: The flag names the CLI

- **WHEN** `proton-sync doctor --proton-cli /opt/proton/proton-drive` runs
- **THEN** the `proton-drive` check runs `/opt/proton/proton-drive`, whatever the config file says

### Requirement: The exit status says which of the three answers it is

`proton-sync doctor` SHALL exit 0 when every required check is ok. It SHALL exit 1 when any
required check reports a problem. It SHALL exit 3 when no required check reports a problem but at
least one could not be checked. Recommended results SHALL NOT change the exit status. Status 2
stays the usage error every `proton-sync` verb returns.

#### Scenario: Everything required is ok

- **WHEN** every required check is ok and `curl` is missing
- **THEN** `proton-sync doctor` exits 0
- **AND** the `curl` problem is still printed

#### Scenario: A required problem

- **WHEN** `signed-in` reports that no sign-in is stored
- **THEN** `proton-sync doctor` exits 1

#### Scenario: A required check could not be checked

- **WHEN** every required check is ok except `signed-in`, which could not be checked
- **THEN** `proton-sync doctor` exits 3

### Requirement: Human output by default, JSON on request

By default `proton-sync doctor` SHALL print one line per check: the result, the check's name and a
short statement. Each problem or could-not-check result is followed by its fix and links, and a
closing line says which of the three answers applies. With `--json` it SHALL print one JSON object
holding every check's id, severity, result, statement, fix and links, and the overall answer.

#### Scenario: JSON for a script

- **WHEN** `proton-sync doctor --json` runs
- **THEN** stdout is a single JSON object that a JSON parser accepts
- **AND** each check's result is one of `ok`, `missing`, `failed` or `unknown`, where `missing` and
  `failed` are the two kinds of problem and `unknown` is could not check

### Requirement: The desktop app's first run blocks on a required problem

Before first run, the desktop app SHALL run the same check, with its own configured CLI path. It
SHALL block first run when any required check reports a problem, and SHALL then show every result
that is not ok, required problems first, each with its fix and link. It SHALL NOT block on a
could-not-check result or on a recommended problem.

#### Scenario: The CLI is missing at first run

- **WHEN** the desktop app starts for the first time and `proton-drive` is missing
- **THEN** first run is blocked on a screen that lists the missing CLI with its fix
- **AND** the screen has an "Installation help" button that opens `https://proton.me/drive/download`
- **AND** the screen has a "Check again" button that runs the check again

#### Scenario: Signed out at first run

- **WHEN** the CLI works but no sign-in is stored
- **THEN** first run is blocked with the instruction to run `proton-drive auth login`
- **AND** pressing "Check again" after signing in lets first run continue

#### Scenario: A CLI that crashes is not called "not installed"

- **WHEN** `proton-drive version` exits non-zero at first run
- **THEN** the screen says the CLI was found but does not run, with its fix
- **AND** it does not say the CLI is not installed

#### Scenario: Quoted commands are drawn as code

- **WHEN** a statement or fix quotes a command or name in backticks
- **THEN** the screen draws it as a code span and shows no backtick
- **AND** `proton-sync doctor` prints the backticks as they are

#### Scenario: Credentials outside the keyring do not block

- **WHEN** `PROTON_DRIVE_CREDENTIALS_STORE` is `pass` and every other required check is ok
- **THEN** first run is not blocked

#### Scenario: Links are opened by name, not by URL

- **WHEN** the user presses a link on the screen
- **THEN** the URL that opens is taken from the application's own list of links
- **AND** no URL is passed from the page to the opener

### Requirement: The expired-sign-in state says how to sign in

When the desktop app shows that Proton is asking the user to sign in again, it SHALL give the
command `proton-drive auth login`, say to run it in a terminal, and offer a link to Proton's CLI
documentation.

#### Scenario: The session expires after the first sync

- **WHEN** the daemon reports that the Proton session was refused
- **THEN** the main screen shows `proton-drive auth login` and a link that opens
  `https://proton.me/support/drive-cli`
- **AND** "Try again now" is still offered

### Requirement: setup.sh checks before it installs the service

In terminal mode, after it builds and writes the config and before it installs or enables the
user service, `setup.sh` SHALL run the `proton-sync doctor` it just installed, against the config
it wrote. It SHALL show the report and act on the exit status. It SHALL NOT carry a second copy of
the checks in shell.

#### Scenario: A required problem stops setup

- **WHEN** `doctor` exits 1
- **THEN** `setup.sh` prints the report, does not install or enable the service, and exits non-zero

#### Scenario: The command setup runs parses

- **WHEN** `setup.sh` runs the check on a machine where every required check is ok
- **THEN** the `proton-sync` command line it builds is accepted, and `doctor` exits 0
- **AND** setup continues

#### Scenario: Could not check is a warning

- **WHEN** `doctor` exits 3
- **THEN** `setup.sh` prints the report and a warning, and continues

#### Scenario: The override

- **WHEN** `setup.sh` is run with `--ignore-checks` and `doctor` exits 1
- **THEN** `setup.sh` prints the report, says it is continuing because of `--ignore-checks`, and
  continues to the dry-run preview

#### Scenario: An installed proton-sync that predates doctor

- **WHEN** `setup.sh --no-build` finds a `proton-sync` that has no `doctor` verb
- **THEN** `setup.sh` stops and says the installed `proton-sync` is too old to check, instead of
  reading the usage error as a result

#### Scenario: Desktop-app mode reports and continues

- **WHEN** `setup.sh --gui` runs and `doctor` exits 1
- **THEN** `setup.sh` prints the report, says the app will run the same check, and continues to
  build and open the app

#### Scenario: A CLI given to setup is checked in either mode

- **WHEN** `setup.sh --gui --proton-cli /opt/proton-drive` runs
- **THEN** the check it runs is `proton-sync doctor --proton-cli /opt/proton-drive`, with no
  `--config`

### Requirement: The daemon reports a missing dependency at start-up and keeps running

At start-up the daemon SHALL check that its configured CLI runs in its own environment and, when
change detection is on for any folder pair, that `curl` is installed. It SHALL log each problem
once, with its fix and docs link, and SHALL keep running. A sign-in problem is reported by the
daemon's existing sign-in state.

#### Scenario: The service cannot find the CLI

- **WHEN** `proton-syncd` starts and its `proton_cli` cannot be found on the service's `PATH`
- **THEN** the log has one warning naming the missing CLI, its fix and the docs link
- **AND** the daemon keeps running and answers on its control socket

#### Scenario: Reported once, not once per pass

- **WHEN** the daemon keeps running with the CLI still missing
- **THEN** the start-up warning is not repeated by later passes

#### Scenario: Shutdown ends the start-up check

- **WHEN** the daemon is told to stop while `proton-drive version` is still running
- **THEN** the child is killed and reaped, and the daemon does not exit until the check has ended
  (for a short, bounded wait)
- **AND** no `proton-drive` child started by the check outlives the daemon

#### Scenario: A signed-out session names the right command

- **WHEN** a pass is refused because the Proton session is signed out or expired
- **THEN** the reason the daemon reports and `proton-sync status` prints contain
  `proton-drive auth login`

### Requirement: The wrong sign-in command does not come back

The repository's code, tests, scripts, desktop app and docs site SHALL NOT give the sign-in command
as `proton-drive login`. Dated design records under `docs/` and planning files under `openspec/`
are exempt. A test SHALL fail when the old spelling appears outside them, including when it is
split across a line break.

#### Scenario: The old spelling is reintroduced across a line break

- **WHEN** a docs page wraps a sentence so that `proton-drive` ends one line and `login` begins the
  next
- **THEN** `cargo test` fails, naming the file and line
