//! Layout guards for the launcher markup.
//!
//! The journal paints one `<div class="entry KIND">` per entry, where KIND mirrors the
//! journal entry kind. The window container used to be `.shell` too, so the first shell
//! command in the journal inherited `position: fixed; inset: 0` and covered the whole
//! window — the UI looked stuck behind one entry. These tests keep that from coming back.

use std::collections::BTreeSet;

const INDEX: &str = include_str!("../../ui/index.html");

/// Classes the journal can put on an entry (mirrors the GLYPH map in the markup).
const KINDS: &[&str] = &[
    "stage", "step", "result", "error", "note", "shell", "gui", "tunnel", "ui", "key", "admin",
    "agent", "relay",
];

#[test]
fn the_window_container_is_not_named_after_a_journal_kind() {
    assert!(INDEX.contains("<div class=\"app-shell\">"), "window container must be .app-shell");
    assert!(
        !INDEX.contains("\n  .shell {"),
        "a top-level .shell rule would also match journal entries with the shell kind"
    );
}

#[test]
fn every_journal_kind_has_a_rule_and_no_full_window_geometry() {
    for kind in KINDS {
        let rule = format!(".entry.{kind}");
        assert!(INDEX.contains(&rule), "no styling for journal kind {kind}");
    }

    // Only the container may be pinned to the window; entries are laid out inline.
    for line in INDEX.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('.') && !trimmed.starts_with(".app-shell") {
            assert!(
                !trimmed.contains("position: fixed") && !trimmed.contains("inset: 0"),
                "full-window geometry must live on the container only: {trimmed}"
            );
        }
    }
}

#[test]
fn the_pairing_block_cannot_be_resized_by_the_user() {
    // The resize grip in WebView2 renders as a white square in the dark card.
    assert!(INDEX.contains("resize: none"));
    assert!(INDEX.contains("textarea::-webkit-resizer { display: none; }"));
}

#[test]
fn the_status_grid_lists_only_the_tiles_the_user_asked_for() {
    for id in ["sHost", "sCounters", "sAdmin"] {
        assert!(!INDEX.contains(id), "{id} was removed from the launcher on purpose");
    }
    let ids: BTreeSet<&str> = ["sTunnel", "sPort", "sUptime"].into_iter().collect();
    for id in ids {
        assert!(INDEX.contains(id), "status tile {id} is missing");
    }
}
