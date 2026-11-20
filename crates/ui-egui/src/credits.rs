//! The About window's credits: every contributor (GitHub login always; display and real names only
//! with recorded consent) and the AI models that helped. The tables are baked into the binary by
//! build.rs from contributors/contributors.json; nothing is
//! read at run time.

use std::borrow::Cow;

use craft::ui::patterns::form_dialogs::{
    AboutInfo, AboutLayout, AboutLink, AboutTab, Contributor as ContributorCredit, Credits, HostedAbout, ModelCredit,
};

use crate::language_packs::language_data_tab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contributor {
    pub login: &'static str,
    pub display_name: Option<&'static str>,
    pub real_name: Option<&'static str>,
    pub prs: u64,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub binary_added: u64,
    pub binary_deleted: u64,
    /// ISO 8601 UTC; empty for someone credited only through a merged PR.
    pub first_commit: &'static str,
    pub last_commit: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub company: &'static str,
    pub model: &'static str,
    pub version: &'static str,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
}

include!(concat!(env!("OUT_DIR"), "/credits.rs"));

/// The baked contributor and model tables as the About dialog's credits. A name is only present where
/// build.rs recorded consent for it; the profile link is the contributor's GitHub page.
pub fn credits() -> Credits {
    let contributors: Vec<ContributorCredit> = CONTRIBUTORS
        .iter()
        .map(|c| {
            let mut credit = ContributorCredit::new(c.login)
                .profile(Cow::Owned(format!("https://github.com/{}", c.login)))
                .counts(c.prs, c.commits)
                .lines(c.lines_added, c.lines_deleted)
                .binary(c.binary_added, c.binary_deleted)
                .active(c.first_commit, c.last_commit);
            if let Some(name) = c.display_name {
                credit = credit.display_name(name);
            }
            if let Some(name) = c.real_name {
                credit = credit.real_name(name);
            }
            credit
        })
        .collect();
    let models: Vec<ModelCredit> =
        MODELS.iter().map(|m| ModelCredit::new(m.company, m.model, m.version).work(m.commits, m.lines_added, m.lines_deleted)).collect();
    Credits::new(contributors, models, TOTAL_COMMITS)
}

/// The About tab's contents: name, blurb, version and build, and the project links in the old About's order (site,
/// Discord, source), one to a line.
pub fn about_info() -> AboutInfo {
    let build = option_env!("WORDCRAFT_BUILD_SHA").map(|s| s.get(..8).unwrap_or(s)).unwrap_or(tl!("development build"));
    // The dialog's own "Version {version}" words the line, so this is the number and the build only.
    let version = crate::i18n::fmt(tl!("{version} ({build})"), &[("version", env!("CARGO_PKG_VERSION")), ("build", build)]);
    AboutInfo::new(
        tl!("WordCraft").to_string(),
        tl!("A free, open-source word processor written from scratch in Rust.\nPart of the Crafting Apps from the ArtCraft team.").to_string(),
        version.to_string(),
    )
    .links([
        AboutLink::new(tl!("getartcraft.com/apps/wordcraft").to_string(), "https://getartcraft.com/apps/wordcraft"),
        AboutLink::new(tl!("Join us on Discord: discord.gg/artcraft").to_string(), "https://discord.gg/artcraft"),
        AboutLink::new(tl!("Source code: github.com/storytold/wordcraft").to_string(), "https://github.com/storytold/wordcraft"),
    ])
    .built_with(tl!("MIT OR Apache-2.0. Copyright (c) 2026 ArtCraft Team and the WordCraft contributors.").to_string())
}

/// The About dialog opened on `tab`, with the Language data tab after Models (`AboutTab::App(0)`). The name
/// table of the dialogs opens it for `about`, `contributors`, `models` and `languageData`.
pub(crate) fn about(tab: AboutTab) -> HostedAbout {
    // PLAN: assumed `HostedAbout::layout(self, AboutLayout) -> Self` (look-dialogs-b framework fix)
    HostedAbout::new(about_info(), credits()).app_tab(language_data_tab()).tab(tab).layout(AboutLayout::Listed)
}
