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

const CONF: &str = include_str!("../tauri.conf.json");
const SPLASH: &str = include_str!("splash.rs");

/// Launcher size the splash grows into.
const SPLASH_WIDTH: f64 = 940.0;
const SPLASH_HEIGHT: f64 = 760.0;

/// Pulls the declarations of one CSS rule so the tests can assert on them.
fn rule(selector: &str) -> String {
    let start = INDEX
        .find(&format!("\n  {selector} {{"))
        .unwrap_or_else(|| panic!("no CSS rule for {selector}"));
    let rest = &INDEX[start..];
    let end = rest.find('}').expect("unterminated CSS rule");
    rest[..end].to_string()
}

#[test]
fn only_the_journal_scrolls() {
    // The window used to scroll as a whole, and the pairing block had its own scrollbar
    // on top of that. Now the journal is the single scroller.
    let content = rule(".content");
    assert!(content.contains("overflow: hidden"), "the main column must not scroll");

    let journal = rule(".journal");
    assert!(journal.contains("overflow-y: auto"), "the journal must scroll on its own");
}

#[test]
fn the_pairing_block_fits_its_text_instead_of_scrolling() {
    let textarea = rule("textarea");
    assert!(textarea.contains("overflow: hidden"), "the pairing block must not scroll");
    assert!(textarea.contains("resize: none"));

    // Height is fitted in JS after every update, otherwise the box clips the last line.
    assert!(INDEX.contains("function fitPairing()"));
    assert_eq!(
        INDEX.matches("fitPairing();").count(),
        3,
        "fitPairing must run on key toggle, on state refresh and on resize"
    );
}

fn main_window_size() -> (u32, u32) {
    let main = CONF.split("\"label\": \"main\"").nth(1).expect("main window entry");
    let number = |key: &str| {
        main.split(&format!("\"{key}\": "))
            .nth(1)
            .and_then(|rest| rest.split(',').next())
            .and_then(|value| value.trim().parse::<u32>().ok())
            .unwrap_or_else(|| panic!("main window {key}"))
    };
    (number("width"), number("height"))
}

#[test]
fn the_window_starts_as_a_small_rounded_card() {
    let (width, height) = main_window_size();
    assert!(width <= 460 && height <= 300, "startup window must be small, got {width}x{height}");
    assert!(width < SPLASH_WIDTH as u32, "the startup window must be smaller than the launcher");
}

#[test]
fn the_launcher_growth_target_matches_the_stylesheet_window() {
    // splash.rs animates to these numbers; the conf keeps only the startup size.
    assert_eq!(SPLASH_WIDTH, 940.0);
    assert_eq!(SPLASH_HEIGHT, 760.0);
    assert!(SPLASH.contains("pub const FULL_WIDTH: f64 = 940.0"), "splash grew away from the launcher size");
    assert!(SPLASH.contains("pub const FULL_HEIGHT: f64 = 760.0"));
    assert!(SPLASH.contains("shape::apply(&rounded)"), "the new rounded outline must be applied afterwards");
    assert!(INDEX.contains("function finishSplash()"), "the renderer must drive the growth");
    assert!(INDEX.contains("invoke(\"expand_window\")"));
    assert!(
        INDEX.contains("SPLASH_MAX_MS"),
        "the splash must not hang forever when the tunnel never arrives"
    );
}

#[test]
fn the_title_bar_has_no_status_chips_and_larger_buttons() {
    for gone in ["id=\"agentDot\"", "id=\"tunnelDot\"", "id=\"stageChip\"", "class=\"chip\""] {
        assert!(!INDEX.contains(gone), "{gone} was removed from the title bar on purpose");
    }
    let buttons = rule(".tb-btn");
    assert!(buttons.contains("width: 46px"), "window buttons must be larger");
    assert!(buttons.contains("height: 32px"));
}

#[test]
fn the_journal_card_is_called_loggi() {
    assert!(INDEX.contains("<h2>Логги</h2>"));
    assert!(!INDEX.contains("<h2>Журнал действий</h2>"));
}

#[test]
fn the_status_tiles_keep_air_above_their_buttons() {
    assert!(INDEX.contains("class=\"row tiles-row\""));
    let row = rule(".tiles-row");
    assert!(
        row.contains("margin-top: 22px") || row.contains("margin-top:22px"),
        "the button row needs vertical breathing room"
    );
}

#[test]
fn the_growth_animation_is_smooth_by_construction() {
    // One native call per frame: move + resize + re-round at once.
    assert!(
        SPLASH.contains("winapi::set_rect_rounded(hwnd, x, y, width, height, radius)"),
        "each frame must move, resize and re-round in a single call"
    );
    // Paced: the next frame waits until the previous one has been applied.
    assert!(SPLASH.contains("recv_timeout(FRAME_TIMEOUT)"), "frames must be paced by rendezvous");
    // Fast: a fixed, short budget instead of a frame count.
    assert!(SPLASH.contains("Duration::from_millis(340)"));
    // Centred on the monitor the window is on, not on the primary screen.
    assert!(SPLASH.contains("current_monitor()"));
    assert!(SPLASH.contains("origin_x + (screen_width - target_width) / 2"));
    // Corners stay rounded while it grows.
    assert!(
        !SPLASH.contains("shape::clear"),
        "clearing the region during growth is what made the corners square"
    );
}

#[test]
fn the_startup_card_is_rounded_before_it_is_shown() {
    // hidden window -> rounded -> shown by the renderer once it has painted
    assert!(CONF.contains("\"visible\": false"), "the main window must start hidden");
    assert!(INDEX.contains("invoke(\"show_window\")"), "the renderer must reveal the window");
    let main = include_str!("main.rs");
    assert!(main.contains("shape::apply(&main)"), "rounding must happen before the first show");
    assert!(main.contains("splash::round_now"), "the region must be re-applied after painting");
}
