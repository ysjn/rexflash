//! Built-in and user patterns. A `(?P<match>...)` group limits what is highlighted and copied.
//!
//! The `regex` crate has no look-around, so a path-like pattern starts with a boundary group
//! (`B`) that is not part of the match. This also stops a path from starting with `-`.

use crate::screen::Line;
use regex::Regex;
use unicode_width::UnicodeWidthChar;

/// Extra test on a regex match, for rules a regex cannot express.
type Check = fn(&str) -> bool;

pub struct Pattern {
    re: Regex,
    weight: u8,
    trim: bool,
    check: Option<Check>,
    url: bool,
}

pub struct Found {
    pub line: usize,
    /// Character index of the first character.
    pub start: usize,
    /// Character index after the last character.
    pub end: usize,
    pub text: String,
    /// OSC 8 destinations are used verbatim, including when opening them.
    pub is_uri: bool,
    pub weight: u8,
}

pub const DEFAULT: &[&str] = &[
    "url", "file-line", "path", "sha", "ip", "ipv6", "uuid", "email", "hex", "color", "semver",
    "diff", "git-status", "jwt", "base64", "base32",
];

/// What may come before a path-like match.
const B: &str = r#"(?:^|[\s"'`(\[{<=,;])"#;

fn has_digit(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_digit())
}

fn is_sha(s: &str) -> bool {
    has_digit(s) && s.bytes().any(|b| (b'a'..=b'f').contains(&b))
}

fn is_ipv4(s: &str) -> bool {
    let host = s.split(':').next().unwrap_or(s);
    host.split('.').all(|o| o.parse::<u16>().is_ok_and(|n| n <= 255))
}

fn is_ipv6(s: &str) -> bool {
    has_digit(s) && (s.contains("::") || s.matches(':').count() == 7)
}

/// Base64 with `=` padding, or long enough and mixed enough that it is not a word or an identifier.
fn is_base64(s: &str) -> bool {
    let body = s.trim_end_matches('=');
    let upper = body.bytes().any(|b| b.is_ascii_uppercase());
    let lower = body.bytes().any(|b| b.is_ascii_lowercase());
    if body.len() < s.len() {
        return s.len().is_multiple_of(4) && (has_digit(body) || (upper && lower));
    }
    // A leading `/` is an absolute path.
    s.len() >= 24 && upper && lower && has_digit(s) && !s.starts_with('/')
}

/// Base32 (RFC 4648) with `=` padding, or long and with a digit, so that `CONSTANT_NAME` does not match.
fn is_base32(s: &str) -> bool {
    s.len().is_multiple_of(8) && (s.ends_with('=') || (s.len() >= 16 && has_digit(s)))
}

fn builtin(name: &str) -> Option<Pattern> {
    let plain = |body: &str| format!("(?P<match>{body})");
    let bounded = |body: &str| format!("{B}(?P<match>{body})");
    let (weight, src, trim, check): (u8, String, bool, Option<Check>) = match name {
        "url" => (
            4,
            plain(r#"(?:https?|ftp|ssh|git|file)://[^\s<>"'`)\]}]+|git@[\w.\-]+:[\w./~\-]+"#),
            true,
            None,
        ),
        "file-line" => (
            5,
            bounded(
                r"(?:~|\.{1,2})?/?(?:[\w.@+][\w.@+\-]*/)*[\w@+][\w.@+\-]*\.[A-Za-z][A-Za-z0-9]*:\d+(?::\d+)?",
            ),
            false,
            None,
        ),
        "path" => (
            4,
            bounded(
                r"(?:~|\.{1,2})?/(?:[\w.@+\-]+/?)+|[\w@+][\w.@+\-]*(?:/[\w.@+\-]+)+/?",
            ),
            true,
            None,
        ),
        "sha" => (3, plain(r"\b[0-9a-f]{7,64}\b"), false, Some(is_sha)),
        "ip" => (1, plain(r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d{1,5})?\b"), false, Some(is_ipv4)),
        "ipv6" => (
            1,
            bounded(r"(?:[0-9a-fA-F]{0,4}:){2,7}[0-9a-fA-F]{1,4}\b"),
            false,
            Some(is_ipv6),
        ),
        "uuid" => (
            1,
            plain(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b"),
            false,
            None,
        ),
        "email" => (1, plain(r"\b[\w.+\-]+@[\w\-]+(?:\.[\w\-]+)+\b"), false, None),
        "hex" => (1, plain(r"\b0x[0-9a-fA-F]+\b"), false, None),
        "color" => (1, plain(r"#(?:[0-9a-fA-F]{6}|[0-9a-fA-F]{3})\b"), false, None),
        "semver" => (
            1,
            plain(r"\bv?\d+\.\d+\.\d+(?:-[\w.]+)?(?:\+[\w.]+)?\b"),
            false,
            None,
        ),
        "digit" => (1, plain(r"\b\d{4,}\b"), false, None),
        // Each part may carry `=` padding. The header starts with `eyJ`, which is `{"` in base64.
        "jwt" => (
            4,
            plain(r"\beyJ[A-Za-z0-9_-]+={0,2}\.[A-Za-z0-9_-]+={0,2}\.[A-Za-z0-9_-]*={0,2}"),
            false,
            None,
        ),
        "base64" => (2, bounded(r"[A-Za-z0-9+/]{8,}(?:={1,2}|\b)"), false, Some(is_base64)),
        "base32" => (2, bounded(r"[A-Z2-7]{8,}(?:={1,6}|\b)"), false, Some(is_base32)),
        "diff" => (
            3,
            r"^(?:(?:---|\+\+\+) [ab]/|diff --git a/)(?P<match>\S+)".to_string(),
            false,
            None,
        ),
        "git-status" => (
            3,
            r"^\s*(?:modified|deleted|new file|renamed|copied|typechange|both modified|both added|deleted by us|deleted by them|added by us|added by them):\s+(?P<match>\S+)"
                .to_string(),
            false,
            None,
        ),
        _ => return None,
    };
    Some(Pattern { re: Regex::new(&src).expect("built-in pattern"), weight, trim, check, url: name == "url" })
}

/// `names` selects built-ins; `None` selects `DEFAULT`. `extra` are user regexes. An unknown name is an error.
pub fn build(names: Option<&[String]>, extra: &[String]) -> Result<Vec<Pattern>, String> {
    let mut out = Vec::new();
    match names {
        Some(names) => {
            for n in names {
                out.push(builtin(n).ok_or_else(|| format!("unknown pattern {n:?}"))?);
            }
        }
        None => {
            for n in DEFAULT {
                out.push(builtin(n).expect("default pattern"));
            }
        }
    }
    for e in extra {
        let re = Regex::new(e).map_err(|err| format!("extra pattern {e:?}: {err}"))?;
        out.push(Pattern { re, weight: 2, trim: false, check: None, url: false });
    }
    Ok(out)
}

/// Restore indented TUI URL continuations without changing their screen coordinates.
pub fn join_wrapped_urls(patterns: &[Pattern], lines: Vec<Line>, cols: usize) -> Vec<Line> {
    let Some(url) = patterns.iter().find(|p| p.url) else { return lines };
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    for line in lines {
        let text = line.text.trim_start_matches(' ');
        let indent = line.text.len() - text.len();
        let previous = out.last_mut().filter(|previous| {
            // shortcut: Infer only indented hard wraps at the right edge; use TUI link metadata when available.
            let at_edge = previous.pos.last().zip(previous.text.chars().last())
                .is_some_and(|(&(_, col), ch)| col + ch.width().unwrap_or(0) == cols);
            if indent < 2 || !at_edge || !previous.hyperlinks.is_empty() || !line.hyperlinks.is_empty() {
                return false;
            }
            let Some(tail) = url.re.find_iter(&previous.text).last().filter(|m| m.end() == previous.text.len()) else {
                return false;
            };
            if indent > previous.pos[previous.char_index(tail.start())].1
                || url.re.find(text).is_some_and(|m| m.start() == 0)
            {
                return false;
            }
            let combined = format!("{}{text}", tail.as_str());
            let Some(matched) = url.re.find(&combined) else { return false };
            let added = matched.end() - tail.len();
            if added == 0 {
                return false;
            }
            let (fragment, suffix) = text.split_at(added);
            suffix.trim().is_empty() || !suffix.starts_with(char::is_whitespace)
                || fragment.contains(['/', '?', '#', '&', '=', '%'])
                || tail.as_str().ends_with(['/', '?', '#', '&', '=', '%'])
        });
        if let Some(previous) = previous {
            let offset = previous.text.len();
            previous.text.push_str(text);
            previous.byte.extend(line.byte[indent..].iter().map(|b| offset + b - indent));
            previous.pos.extend_from_slice(&line.pos[indent..]);
        } else {
            out.push(line);
        }
    }
    out
}

/// Finds hyperlinks and pattern matches. Hyperlinks take precedence over overlapping patterns;
/// other overlaps resolve to the leftmost match, then the longest.
pub fn find_all(patterns: &[Pattern], lines: &[Line]) -> Vec<Found> {
    let mut out = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let mut found: Vec<Found> = Vec::new();
        for p in patterns {
            for caps in p.re.captures_iter(&line.text) {
                let Some(m) = caps.name("match").or_else(|| caps.get(0)) else {
                    continue;
                };
                let mut text = m.as_str();
                if p.trim {
                    text = text.trim_end_matches(['.', ',', ';', ':', '!', '?']);
                }
                if text.is_empty() || p.check.is_some_and(|check| !check(text)) {
                    continue;
                }
                let start = line.char_index(m.start());
                let end = line.char_index(m.start() + text.len());
                if line.hyperlinks.iter().any(|link| start < link.end && link.start < end) {
                    continue;
                }
                found.push(Found {
                    line: li,
                    start,
                    end,
                    text: text.to_string(),
                    is_uri: false,
                    weight: p.weight,
                });
            }
        }
        found.extend(line.hyperlinks.iter().map(|link| Found {
            line: li,
            start: link.start,
            end: link.end,
            text: link.uri.clone(),
            is_uri: true,
            weight: 5,
        }));
        found.sort_by(|a, b| a.start.cmp(&b.start).then((b.end - b.start).cmp(&(a.end - a.start))));
        let mut last = 0;
        for f in found {
            if f.start >= last {
                last = f.end;
                out.push(f);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Screen;

    fn matches(input: &str, names: Option<&[&str]>) -> Vec<String> {
        let screen = Screen::parse(input, 200);
        let lines = screen.lines();
        let names: Option<Vec<String>> = names.map(|n| n.iter().map(|s| s.to_string()).collect());
        let patterns = build(names.as_deref(), &[]).unwrap();
        find_all(&patterns, &lines).into_iter().map(|f| f.text).collect()
    }

    #[test]
    fn url_drops_trailing_punctuation() {
        assert_eq!(
            matches("see https://example.com/a?b=1, ok\n", Some(&["url"])),
            vec!["https://example.com/a?b=1"]
        );
    }

    #[test]
    fn url_wins_over_the_path_inside_it() {
        assert_eq!(matches("https://example.com/a/b\n", None), vec!["https://example.com/a/b"]);
    }

    #[test]
    fn file_line_wins_over_path() {
        assert_eq!(matches("at ./src/main.rs:42:5 now\n", None), vec!["./src/main.rs:42:5"]);
    }

    #[test]
    fn absolute_path_matches() {
        assert_eq!(matches("cd /usr/local/bin now\n", Some(&["path"])), vec!["/usr/local/bin"]);
    }

    #[test]
    fn path_never_starts_with_a_dash() {
        assert_eq!(matches("-a/Some.app\n", Some(&["path"])), Vec::<String>::new());
    }

    #[test]
    fn sha_needs_a_digit_and_a_hex_letter() {
        assert_eq!(
            matches("abc1234def deadbeef 1234567\n", Some(&["sha"])),
            vec!["abc1234def"]
        );
    }

    #[test]
    fn ipv4_keeps_the_port_and_rejects_big_octets() {
        assert_eq!(
            matches("192.168.0.1:8080 and 999.1.1.1\n", Some(&["ip"])),
            vec!["192.168.0.1:8080"]
        );
    }

    #[test]
    fn ipv6_needs_double_colon_or_eight_groups() {
        assert_eq!(matches("::1 12:34:56\n", Some(&["ipv6"])), vec!["::1"]);
    }

    #[test]
    fn uuid_email_hex_color_semver() {
        assert_eq!(
            matches(
                "123e4567-e89b-12d3-a456-426614174000 a@b.co 0xdeadBEEF #1a2b3c v1.2.3-rc.1\n",
                Some(&["uuid", "email", "hex", "color", "semver"])
            ),
            vec![
                "123e4567-e89b-12d3-a456-426614174000",
                "a@b.co",
                "0xdeadBEEF",
                "#1a2b3c",
                "v1.2.3-rc.1"
            ]
        );
    }

    #[test]
    fn git_status_and_diff_copy_only_the_path() {
        assert_eq!(
            matches("\tmodified:   src/lib.rs\n+++ b/src/new.rs\n", Some(&["git-status", "diff"])),
            vec!["src/lib.rs", "src/new.rs"]
        );
    }

    #[test]
    fn base64_keeps_the_padding_and_wins_over_path() {
        assert_eq!(
            matches("d5VsWZyhpBEun7usAa7QMnIN5c6WYS/EP7Caj/66OLo=\n", None),
            vec!["d5VsWZyhpBEun7usAa7QMnIN5c6WYS/EP7Caj/66OLo="]
        );
        assert_eq!(matches("key=dGVzdDEyMw==\n", Some(&["base64"])), vec!["dGVzdDEyMw=="]);
    }

    #[test]
    fn base64_ignores_words_and_paths() {
        assert_eq!(
            matches("InitializeConfiguration /usr/local/lib/Python3Packages/Thing\n", Some(&["base64"])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn base32_keeps_the_padding() {
        assert_eq!(matches("JBSWY3DPEE======\n", Some(&["base32"])), vec!["JBSWY3DPEE======"]);
        assert_eq!(matches("CONSTANT_NAME\n", Some(&["base32"])), Vec::<String>::new());
    }

    #[test]
    fn jwt_is_one_match() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(matches(&format!("Bearer {jwt}\n"), None), vec![jwt]);
        assert_eq!(matches("eyJhbGciOiJIUzI1NiJ9==.eyJzdWIiOiIxMjM0NTY3ODkwIn0=.sig=\n", Some(&["jwt"])), vec![
            "eyJhbGciOiJIUzI1NiJ9==.eyJzdWIiOiIxMjM0NTY3ODkwIn0=.sig="
        ]);
    }

    #[test]
    fn digit_is_off_by_default() {
        assert_eq!(matches("pid 123456\n", None), Vec::<String>::new());
        assert_eq!(matches("pid 123456\n", Some(&["digit"])), vec!["123456"]);
    }

    #[test]
    fn match_on_a_wrapped_line_is_one_match() {
        let screen = Screen::parse("see https://example.com/x\n", 16);
        let lines = screen.lines();
        let found = find_all(&build(Some(&["url".to_string()]), &[]).unwrap(), &lines);
        assert_eq!(found[0].text, "https://example.com/x");
        assert_eq!(lines[0].pos[found[0].end - 1], (1, 8));
    }

    #[test]
    fn tui_url_continuations_keep_unicode_and_screen_positions_across_three_rows() {
        let screen = Screen::parse(concat!(
            "\x1b[2C日本語 \x1b[32mhttps://example.com/abc\r\n",
            "  abcdefghijklmnopqrstuvwxyz/123\r\n",
            "  456?x=1&y=2\x1b[0m now\n",
        ), 32);
        let patterns = build(None, &[]).unwrap();
        let lines = join_wrapped_urls(&patterns, screen.lines(), 32);
        assert_eq!(screen.rows.len(), 3);
        assert_eq!(lines.len(), 1);
        let found = find_all(&patterns, &lines);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "https://example.com/abcabcdefghijklmnopqrstuvwxyz/123456?x=1&y=2");
        assert_eq!(lines[0].pos[found[0].start], (0, 9));
        let continuation = lines[0].char_index(lines[0].text.find("456").unwrap());
        assert_eq!(lines[0].char_at(continuation), Some('4'));
        assert_eq!(lines[0].pos[continuation], (2, 2));
        assert_eq!(lines[0].byte.len(), lines[0].pos.len());
    }

    #[test]
    fn tui_url_recovery_does_not_join_other_urls_prose_blank_lines_or_different_indents() {
        let first = "  https://example.com/abc";
        for next in [
            "  https://other.example/path", "  next paragraph", "", "/separate/path",
            "    /deeper/path", "  - another item", "  • another item",
        ] {
            let screen = Screen::parse(&format!("{first}\r\n{next}\r\nend\n"), first.len());
            let lines = join_wrapped_urls(&build(None, &[]).unwrap(), screen.lines(), first.len());
            assert_eq!(lines.len(), 3, "{next:?}");
            assert_eq!(lines[0].text, first, "{next:?}");
        }
    }

    #[test]
    fn tui_url_recovery_requires_the_right_edge_and_an_enabled_url_pattern() {
        let first = "  https://example.com/abc";
        let vt = format!("{first}\r\n  /continued\n");
        for (cols, names) in [
            (first.len() + 1, vec!["url".to_string()]),
            (first.len(), vec!["path".to_string()]),
        ] {
            let screen = Screen::parse(&vt, cols);
            let lines = join_wrapped_urls(&build(Some(&names), &[]).unwrap(), screen.lines(), cols);
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].text, first);
        }
    }

    #[test]
    fn tui_url_recovery_preserves_explicit_hyperlinks() {
        for vt in [
            "  \x1b]8;;https://target.example/\x07https://example.com/abc\x1b]8;;\x07\r\n  /continued\n",
            "  https://example.com/abc\r\n  \x1b]8;;https://target.example/\x07/continued\x1b]8;;\x07\n",
        ] {
            let screen = Screen::parse(vt, 25);
            let patterns = build(None, &[]).unwrap();
            let lines = join_wrapped_urls(&patterns, screen.lines(), 25);
            assert_eq!(lines.len(), 2);
            assert!(find_all(&patterns, &lines).iter().any(|f| f.is_uri && f.text == "https://target.example/"));
        }
    }

    #[test]
    fn unknown_pattern_is_an_error() {
        let err = build(Some(&["kubernetes".to_string()]), &[]).err().unwrap();
        assert_eq!(err, "unknown pattern \"kubernetes\"");
    }

    #[test]
    fn extra_pattern_uses_the_match_group() {
        let screen = Screen::parse("ticket ABC-123 done\n", 80);
        let lines = screen.lines();
        let patterns = build(Some(&[]), &["ticket (?P<match>[A-Z]+-\\d+)".to_string()]).unwrap();
        assert_eq!(find_all(&patterns, &lines)[0].text, "ABC-123");
    }

    #[test]
    fn osc8_is_found_even_without_regex_patterns() {
        assert_eq!(
            matches("\x1b]8;;https://example.com/docs?a=1;b=2#section?\x07仕様書\x1b]8;;\x07", Some(&[])),
            vec!["https://example.com/docs?a=1;b=2#section?"]
        );
    }

    #[test]
    fn osc8_wins_over_patterns_that_start_before_or_inside_its_label() {
        assert_eq!(
            matches("https://shown.example/\x1b]8;;https://target.example/docs\x07path\x1b]8;;\x07 /unlinked/path", None),
            vec!["https://target.example/docs", "/unlinked/path"]
        );
        assert_eq!(
            matches("\x1b]8;;file:///actual/source.rs#L7\x07src/main.rs:42:5\x1b]8;;\x07", None),
            vec!["file:///actual/source.rs#L7"]
        );
    }

    #[test]
    fn osc8_wins_over_a_larger_custom_pattern() {
        let lines = Screen::parse("before \x1b]8;;https://target\x07docs\x1b]8;;\x07 after", 80).lines();
        let patterns = build(Some(&[]), &["before .* after".into()]).unwrap();
        let found = find_all(&patterns, &lines);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].start, found[0].end, found[0].text.as_str()), (7, 11, "https://target"));
        assert!(found[0].is_uri);
    }
}
