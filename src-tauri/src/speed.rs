//! Guards for the things that make the bridge fast.
//!
//! Every one of these was a measurable cost: a full-resolution colour conversion per
//! screenshot, an extra round trip per command, a desktop-wide window enumeration every
//! three seconds, and a journal that rebuilt itself for every new line.

const RELAY: &str = include_str!("relay.rs");
const WINAPI: &str = include_str!("winapi.rs");
const MAIN: &str = include_str!("main.rs");
const STATE: &str = include_str!("state.rs");
const INDEX: &str = include_str!("../../ui/index.html");
const PROMPT: &str = include_str!("../../docs/PAIRING_PROMPT.txt");

#[test]
fn screenshots_are_scaled_inside_gdi_not_in_memory() {
    // A 3440x1440 desktop is 20 MB of pixels; converting all of them for a 1100 px JPEG
    // was the slowest thing the bridge did.
    assert!(
        WINAPI.contains("StretchBlt("),
        "scaling must happen in GDI while copying the screen"
    );
    assert!(WINAPI.contains("CreateCompatibleBitmap(screen, target_width, target_height)"));
    assert!(
        !WINAPI.contains("(width as usize) * (height as usize) * 4"),
        "the pixel buffer must be the size of the scaled bitmap"
    );
}

#[test]
fn screenshots_have_a_size_and_a_quality_knob() {
    assert!(WINAPI.contains("pub fn capture(max_width: u32, quality: u8)"));
    assert!(WINAPI.contains("JpegEncoder::new_with_quality"));
    assert!(RELAY.contains("fn shot_params(payload: &Value) -> (u32, u8)"));
    assert!(RELAY.contains("clamp(320, 3840)"));
    assert!(RELAY.contains("clamp(30, 95)"));
}

#[test]
fn several_commands_share_one_round_trip() {
    assert!(RELAY.contains("\"batch\" => run_batch_job"));
    assert!(RELAY.contains(".take(20)"), "a batch must stay bounded");
    assert!(PROMPT.contains("\"type\":\"batch\""));
}

#[test]
fn state_can_be_read_in_one_request() {
    assert!(RELAY.contains(".route(\"/board\", get(board))"));
    assert!(PROMPT.contains("`/board` instead of `/status` + `/journal` + `/windows`"));
}

#[test]
fn windows_are_not_enumerated_on_a_timer() {
    assert!(
        !MAIN.contains("worker.refresh_windows()"),
        "the three-second loop must not enumerate the desktop any more"
    );
    assert!(STATE.contains("fn windows_recent(&self, max_age"));
    assert!(RELAY.contains("windows_recent(std::time::Duration::from_millis(700))"));
}

#[test]
fn the_journal_appends_instead_of_rebuilding() {
    assert!(
        INDEX.contains("box.insertAdjacentHTML(\"beforeend\", entryHtml(entry))"),
        "one DOM node per entry"
    );
    assert!(INDEX.contains("while (box.childElementCount > JOURNAL_MAX_NODES)"));
    assert!(INDEX.contains("function atBottom(box)"), "scrolling up must not be stolen");
}

#[test]
fn the_prompt_teaches_the_agent_to_waste_fewer_requests() {
    for rule in [
        "Doing the work fast",
        "Group independent commands",
        "Screenshot deliberately",
        "Ask for state once",
        "One request per task",
        "Trust exit codes",
    ] {
        assert!(PROMPT.contains(rule), "missing speed rule: {rule}");
    }
}

#[test]
fn the_bridge_reports_its_own_time() {
    assert!(RELAY.contains("\"took_ms\""), "answers must say how long the bridge took");
    assert!(PROMPT.contains("took_ms"));
}

#[test]
fn a_journal_line_can_ride_along_with_a_command() {
    assert!(RELAY.contains("payload.get(\"note\")"));
    assert!(RELAY.contains("note_kind"));
    assert!(PROMPT.contains("\"note\":\"создана папка"), "the prompt must show the one-request form");
}

#[test]
fn there_is_a_cheap_latency_probe() {
    assert!(RELAY.contains(".route(\"/ping\", get(ping))"));
}

#[test]
fn the_prompt_says_where_the_time_actually_goes() {
    for rule in [
        "Do not follow it with a separate",
        "Trust exit codes",
        "number of steps decides the wall clock",
        "Pack work together",
    ] {
        assert!(PROMPT.contains(rule), "missing rule: {rule}");
    }
}

#[test]
fn the_prompt_forbids_the_base64_workaround() {
    assert!(PROMPT.contains("Base64 tricks are no longer needed"));
    assert!(PROMPT.contains("do not do this any more"));
}
