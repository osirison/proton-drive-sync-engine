//! The dependency check behind `proton-sync doctor`, the daemon's start-up log, `setup.sh` and the
//! desktop app's first-run screen.
//!
//! Only [`links`] is here so far; the check types, the `Environment` seam and the checks
//! themselves arrive with the next part of the change (`openspec/changes/add-dependency-check`).

/// Every URL and docs anchor the check may show a person, in one place. A link spelled anywhere
/// else is a second copy that can drift, so callers take it from here.
///
/// `tests/doctor_links.rs` holds the anchors to the troubleshooting page: each one must be the
/// slug of exactly one heading there.
pub mod links {
    /// Proton's download page for the `proton-drive` CLI.
    pub const PROTON_DOWNLOAD: &str = "https://proton.me/drive/download";
    /// Proton's direct download index.
    pub const PROTON_DIRECT_DOWNLOAD: &str = "https://proton.me/download/drive/cli/index.html";
    /// Proton's support page for the CLI.
    pub const PROTON_SUPPORT: &str = "https://proton.me/support/drive-cli";
    /// The CLI's README, which documents its commands.
    pub const PROTON_README: &str =
        "https://github.com/ProtonDriveApps/sdk/blob/main/cli/README.md";

    /// The command that signs the CLI in.
    pub const SIGN_IN_COMMAND: &str = "proton-drive auth login";

    /// The docs site (the default `site` + `base` of `website/astro.config.mjs`).
    pub const DOCS_SITE: &str = "https://osirison.github.io/proton-drive-sync-engine";

    /// The docs page every check links to, relative to [`DOCS_SITE`].
    pub const TROUBLESHOOTING_PAGE: &str = "/reference/troubleshooting/";

    /// One troubleshooting anchor per check id. A heading a check links to uses only ASCII
    /// letters, digits, spaces and hyphens, so its slug is the heading lower-cased with each space
    /// made a hyphen, whichever slugger the site uses.
    pub const ANCHORS: [(&str, &str); 7] = [
        ("proton-drive", "proton-drive-is-missing-or-does-not-run"),
        ("signed-in", "no-proton-drive-sign-in-is-stored"),
        ("config", "the-config-file-cannot-be-read"),
        ("secret-tool", "secret-tool-is-missing"),
        ("curl", "curl-is-missing"),
        ("session-bus", "no-session-bus"),
        ("service", "the-proton-syncd-service-is-not-installed"),
    ];

    /// The full docs URL for a check id, or `None` for an id with no anchor.
    pub fn docs_url(check: &str) -> Option<String> {
        ANCHORS
            .iter()
            .find(|(id, _)| *id == check)
            .map(|(_, anchor)| format!("{DOCS_SITE}{TROUBLESHOOTING_PAGE}#{anchor}"))
    }
}

#[cfg(test)]
mod tests {
    use super::links;

    #[test]
    fn a_docs_url_joins_the_site_the_page_and_the_anchor() {
        assert_eq!(
            links::docs_url("signed-in").as_deref(),
            Some(
                "https://osirison.github.io/proton-drive-sync-engine/reference/troubleshooting/#no-proton-drive-sign-in-is-stored"
            )
        );
        assert_eq!(links::docs_url("no-such-check"), None);
    }
}
