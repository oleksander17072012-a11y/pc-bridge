//! Turning console output into text you can read.
//!
//! On a Russian Windows `cmd.exe` and `powershell` write their output in the OEM code
//! page (CP866) or, from some programs, in the ANSI code page (CP1251) — not UTF-8. The
//! bridge used to decode everything as UTF-8 lossily, so `C:\Users\user\Desktop\шщреужледур`
//! arrived as `C:\Users\user\Desktop\??????`, and the assistant had to base64-encode
//! names to read them back.
//!
//! Two defences, in this order:
//!   1. ask the shell to speak UTF-8 (`chcp 65001` for cmd, `[Console]::OutputEncoding`
//!      for PowerShell) — this makes new output correct, including in nested programs
//!      that honour the console code page;
//!   2. if the bytes are still not valid UTF-8 (an old program that ignores the code
//!      page), pick the code page that produces the most plausible text.

const CP866: [u16; 128] = [
    0x0410, 0x0411, 0x0412, 0x0413, 0x0414, 0x0415, 0x0416, 0x0417,
    0x0418, 0x0419, 0x041A, 0x041B, 0x041C, 0x041D, 0x041E, 0x041F,
    0x0420, 0x0421, 0x0422, 0x0423, 0x0424, 0x0425, 0x0426, 0x0427,
    0x0428, 0x0429, 0x042A, 0x042B, 0x042C, 0x042D, 0x042E, 0x042F,
    0x0430, 0x0431, 0x0432, 0x0433, 0x0434, 0x0435, 0x0436, 0x0437,
    0x0438, 0x0439, 0x043A, 0x043B, 0x043C, 0x043D, 0x043E, 0x043F,
    0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561, 0x2562, 0x2556,
    0x2555, 0x2563, 0x2551, 0x2557, 0x255D, 0x255C, 0x255B, 0x2510,
    0x2514, 0x2534, 0x252C, 0x251C, 0x2500, 0x253C, 0x255E, 0x255F,
    0x255A, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256C, 0x2567,
    0x2568, 0x2564, 0x2565, 0x2559, 0x2558, 0x2552, 0x2553, 0x256B,
    0x256A, 0x2518, 0x250C, 0x2588, 0x2584, 0x258C, 0x2590, 0x2580,
    0x0440, 0x0441, 0x0442, 0x0443, 0x0444, 0x0445, 0x0446, 0x0447,
    0x0448, 0x0449, 0x044A, 0x044B, 0x044C, 0x044D, 0x044E, 0x044F,
    0x0401, 0x0451, 0x0404, 0x0454, 0x0407, 0x0457, 0x040E, 0x045E,
    0x00B0, 0x2219, 0x00B7, 0x221A, 0x2116, 0x00A4, 0x25A0, 0x00A0,
];

const CP1251: [u16; 128] = [
    0x0402, 0x0403, 0x201A, 0x0453, 0x201E, 0x2026, 0x2020, 0x2021,
    0x20AC, 0x2030, 0x0409, 0x2039, 0x040A, 0x040C, 0x040B, 0x040F,
    0x0452, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
    0xFFFD, 0x2122, 0x0459, 0x203A, 0x045A, 0x045C, 0x045B, 0x045F,
    0x00A0, 0x040E, 0x045E, 0x0408, 0x00A4, 0x0490, 0x00A6, 0x00A7,
    0x0401, 0x00A9, 0x0404, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x0407,
    0x00B0, 0x00B1, 0x0406, 0x0456, 0x0491, 0x00B5, 0x00B6, 0x00B7,
    0x0451, 0x2116, 0x0454, 0x00BB, 0x0458, 0x0405, 0x0455, 0x0457,
    0x0410, 0x0411, 0x0412, 0x0413, 0x0414, 0x0415, 0x0416, 0x0417,
    0x0418, 0x0419, 0x041A, 0x041B, 0x041C, 0x041D, 0x041E, 0x041F,
    0x0420, 0x0421, 0x0422, 0x0423, 0x0424, 0x0425, 0x0426, 0x0427,
    0x0428, 0x0429, 0x042A, 0x042B, 0x042C, 0x042D, 0x042E, 0x042F,
    0x0430, 0x0431, 0x0432, 0x0433, 0x0434, 0x0435, 0x0436, 0x0437,
    0x0438, 0x0439, 0x043A, 0x043B, 0x043C, 0x043D, 0x043E, 0x043F,
    0x0440, 0x0441, 0x0442, 0x0443, 0x0444, 0x0445, 0x0446, 0x0447,
    0x0448, 0x0449, 0x044A, 0x044B, 0x044C, 0x044D, 0x044E, 0x044F,
];

/// Rank a decoding: real letters are good, replacement characters and control codes bad.
fn score(text: &str) -> i32 {
    text.chars()
        .map(|symbol| match symbol {
            '\u{FFFD}' => -6,
            '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' => -4,
            '\u{400}'..='\u{4FF}' => 3,          // Cyrillic
            '\u{20}'..='\u{7E}' => 1,            // printable ASCII
            '\n' | '\r' | '\t' => 1,
            _ => 0,
        })
        .sum()
}

fn decode_with(bytes: &[u8], table: &[u16; 128]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if *byte < 0x80 {
                *byte as char
            } else {
                char::from_u32(table[*byte as usize - 0x80] as u32).unwrap_or('\u{FFFD}')
            }
        })
        .collect()
}

/// UTF-8 if possible, otherwise the code page that reads best.
pub fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => {
            let candidates = [decode_with(bytes, &CP866), decode_with(bytes, &CP1251)];
            candidates
                .into_iter()
                .max_by_key(|text| score(text))
                .unwrap_or_default()
        }
    }
}

/// Prefix that makes `cmd.exe` emit UTF-8. Nested programs that honour the console code
/// page (most of them) are fixed by this alone.
pub fn cmd_wrapper(command: &str) -> String {
    format!("chcp 65001>nul & {command}")
}

/// Prefix that makes PowerShell emit UTF-8 both for the console and for pipelines.
pub fn powershell_wrapper(command: &str) -> String {
    format!(
        "$ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $OutputEncoding=[System.Text.Encoding]::UTF8; {command}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_utf8_is_kept_as_is() {
        assert_eq!(decode("шщреужледур".as_bytes()), "шщреужледур");
    }

    #[test]
    fn oem_russian_is_decoded_instead_of_destroyed() {
        // «шщреужледур» in CP866, exactly what cmd.exe produced on the user's machine.
        let oem = "шщреужледур".as_bytes().to_vec();
        let cp866: Vec<u8> = "шщреужледур"
            .chars()
            .map(|symbol| {
                (0x80u8..=0xFF)
                    .find(|byte| CP866[*byte as usize - 0x80] as u32 == symbol as u32)
                    .unwrap_or(b'?')
            })
            .collect();
        assert_ne!(oem, cp866, "the test needs a real code page difference");
        assert_eq!(decode(&cp866), "шщреужледур");
    }

    #[test]
    fn ansi_russian_is_also_recovered() {
        let cp1251: Vec<u8> = "шщреужледур"
            .chars()
            .map(|symbol| {
                (0x80u8..=0xFF)
                    .find(|byte| CP1251[*byte as usize - 0x80] as u32 == symbol as u32)
                    .unwrap_or(b'?')
            })
            .collect();
        assert_eq!(decode(&cp1251), "шщреужледур");
    }

    #[test]
    fn ascii_is_never_mangled() {
        assert_eq!(decode(b"C:\\Users\\user"), "C:\\Users\\user");
    }

    #[test]
    fn both_shells_are_asked_for_utf8() {
        assert!(cmd_wrapper("dir").starts_with("chcp 65001>nul & "));
        assert!(powershell_wrapper("dir").contains("[Console]::OutputEncoding"));
    }
}
