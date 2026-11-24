//! Optional source-file opening in a Neovim pane. Other targets retain macOS Open.

use crate::rex::Target;
use regex::Regex;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use url::Url;

#[derive(Debug)]
struct OpenTarget {
    path: Option<PathBuf>,
    line: Option<u64>,
    column: Option<u64>,
    argument: String,
}

fn absolute(text: &str, cwd: &Path) -> Result<PathBuf, String> {
    let path = if let Some(rest) = text.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").ok_or("open: HOME is not set")?).join(rest)
    } else {
        cwd.join(text)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => { normalized.pop(); }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn scheme(text: &str) -> bool {
    text.split_once(':').is_some_and(|(prefix, _)| {
        prefix.starts_with(|c: char| c.is_ascii_alphabetic())
            && prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    })
}

fn position(text: Option<&str>) -> Result<Option<u64>, String> {
    text.map(|text| text.parse::<u64>().ok().filter(|&n| (1..=9_007_199_254_740_991).contains(&n))
        .ok_or_else(|| "open: line and column must be positive safe integers".to_owned())).transpose()
}

fn target_of(text: &str, cwd: &Path) -> Result<OpenTarget, String> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return Err("open: invalid target".into());
    }
    let external = || OpenTarget { path: None, line: None, column: None, argument: text.to_owned() };
    let suffix = Regex::new(r"^(.*?):([0-9]+)(?::([0-9]+))?$").expect("location pattern");
    let hash = Regex::new(r"^(.*?)#L([0-9]+)(?:C([0-9]+))?$").expect("location pattern");
    let is_file_uri = text.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"));
    let mut fragment = String::new();
    let mut path = text.to_owned();
    if is_file_uri {
        let uri = Url::parse(text).map_err(|e| format!("open: {e}"))?;
        if uri.host_str().is_some_and(|host| host != "localhost") {
            return Ok(external());
        }
        let mut bytes = uri.path().bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                let high = bytes.next().filter(u8::is_ascii_hexdigit).ok_or("open: invalid file URI escape")?;
                let low = bytes.next().filter(u8::is_ascii_hexdigit).ok_or("open: invalid file URI escape")?;
                if high == b'2' && low.eq_ignore_ascii_case(&b'f') {
                    return Err("open: encoded slash in file URI".into());
                }
            }
        }
        fragment = uri.fragment().map(|f| format!("#{f}")).unwrap_or_default();
        path = uri.to_file_path().map_err(|_| "open: invalid file URI")?
            .into_os_string().into_string().map_err(|_| "open: file URI path is not UTF-8")?;
        if path.chars().any(char::is_control) {
            return Err("open: control character in file URI path".into());
        }
    } else if scheme(text) {
        let without_location = suffix.captures(text).map(|c| c[1].to_owned()).unwrap_or_else(|| text.to_owned());
        if text.contains("://") || (!absolute(text, cwd)?.exists() && !absolute(&without_location, cwd)?.exists()) {
            return Ok(external());
        }
    }
    let (mut line, mut column) = (None, None);
    if !absolute(&path, cwd)?.exists()
        && let Some(captures) = hash.captures(&path).or_else(|| suffix.captures(&path))
    {
        line = captures.get(2).map(|m| m.as_str().to_owned());
        column = captures.get(3).map(|m| m.as_str().to_owned());
        path = captures[1].to_owned();
    }
    if let Some(captures) = hash.captures(&fragment) {
        line = captures.get(2).map(|m| m.as_str().to_owned());
        column = captures.get(3).map(|m| m.as_str().to_owned());
    }
    let path = absolute(&path, cwd)?;
    let argument = if is_file_uri { text.to_owned() } else {
        path.to_str().ok_or("open: target path is not UTF-8")?.to_owned()
    };
    Ok(OpenTarget { path: Some(path), line: position(line.as_deref())?, column: position(column.as_deref())?, argument })
}

fn is_code(path: &Path, mime: &str) -> bool {
    let source_mime = mime.starts_with("text/x-") || matches!(mime,
        "application/javascript" | "application/typescript" | "application/json" | "application/xml"
        | "application/x-javascript" | "application/x-shellscript" | "application/x-python" | "application/x-ruby");
    let textual = mime.starts_with("text/") || source_mime
        || matches!(mime, "image/svg+xml" | "application/x-empty" | "inode/x-empty");
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    // shortcut: extensionless text/plain files use macOS Open; add recognized names when needed.
    textual && (source_mime
        || "c h cc hh cpp hpp cxx hxx m mm cs fs fsx vb go rs py pyi rb php java kt kts swift scala sc js jsx mjs cjs ts tsx mts cts vue svelte html htm css scss sass less sh bash zsh fish ps1 lua vim el ex exs erl hrl clj cljs cljc edn hs lhs ml mli r sql graphql gql proto nix json jsonc yaml yml toml xml xsl xslt svg tex typ"
            .split_ascii_whitespace().any(|ext| ext == extension)
        || ["Makefile", "Dockerfile", "Containerfile", "Justfile", "justfile", "Rakefile", "Gemfile", "CMakeLists.txt", ".bashrc", ".zshrc", ".vimrc"].contains(&name))
}

fn nvim_command(nvim: &str, path: &Path, line: Option<u64>, column: Option<u64>) -> Result<Vec<String>, String> {
    let mut command = vec![nvim.to_owned()];
    if let Some(line) = line {
        // LazyVim initializes its viewport after startup commands; repeat the jump when it settles.
        command.push(format!("+lua local function jump() vim.fn.cursor({line},{}); vim.cmd(\"normal! zz\") end; jump(); vim.api.nvim_create_autocmd(\"User\", {{ pattern = \"VeryLazy\", once = true, callback = function() vim.schedule(jump) end }})", column.unwrap_or(1)));
    }
    command.extend(["--".to_owned(), path.to_str().ok_or("open: path is not UTF-8")?.to_owned()]);
    Ok(command)
}

pub fn selection(text: &str, source: &Target, cwd: Option<&str>, nvim: &str) -> Result<Option<Target>, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("open: {e}"))?.join(cwd.unwrap_or(""));
    if !cwd.is_dir() {
        return Err("open: working directory is not a directory".into());
    }
    let target = target_of(text, &cwd)?;
    if let Some(path) = &target.path
        && path.is_file()
    {
        let mime = Command::new("/usr/bin/file").args(["-Lb", "--mime-type", "--"]).arg(path)
            .output().map_err(|e| format!("file: {e}"))?;
        if !mime.status.success() {
            return Err(format!("file: {}", String::from_utf8_lossy(&mime.stderr).trim()));
        }
        if is_code(path, String::from_utf8_lossy(&mime.stdout).trim()) {
            return source.split(&cwd, &nvim_command(nvim, path, target.line, target.column)?).map(Some);
        }
    }
    let output = Command::new("open").arg(&target.argument).current_dir(&cwd)
        .output().map_err(|e| format!("open: {e}"))?;
    if !output.status.success() {
        return Err(format!("open: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(name: &str) -> (PathBuf, PathBuf) {
        let directory = std::env::temp_dir().join(format!("rexflash-open-{}-{name}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("source with 'quotes' $(touch SHOULD_NOT_EXIST).ts");
        fs::write(&source, "const source = 1;\n").unwrap();
        (directory, source)
    }

    #[test]
    fn resolves_relative_paths_and_positions_without_evaluating_shell_text() {
        let (dir, path) = fixture("relative");
        for suffix in [":42:5", "#L42C5"] {
            let target = target_of(&format!("./{}{}", path.file_name().unwrap().to_str().unwrap(), suffix), &dir).unwrap();
            assert_eq!(target.path.as_ref(), Some(&path));
            assert_eq!((target.line, target.column), (Some(42), Some(5)));
            assert_eq!(nvim_command("/opt/homebrew/bin/nvim", &path, target.line, target.column).unwrap().last().unwrap(), path.to_str().unwrap());
        }
        assert!(!dir.join("SHOULD_NOT_EXIST").exists());
        assert_eq!(target_of("a/../source.ts", &dir).unwrap().path, Some(dir.join("source.ts")));
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(target_of("~/example.ts:3", &dir).unwrap().path, Some(home.join("example.ts")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn local_file_uris_decode_names_and_preserve_both_location_formats() {
        let (dir, path) = fixture("uri");
        let uri = Url::from_file_path(&path).unwrap().to_string();
        for suffix in [":42:5", "#L42C5"] {
            let input = format!("{uri}{suffix}");
            let target = target_of(&input, &dir).unwrap();
            assert_eq!(target.path.as_ref(), Some(&path));
            assert_eq!((target.line, target.column), (Some(42), Some(5)));
            assert_eq!(target.argument, input);
        }
        assert_eq!(target_of(&uri.replacen("file:///", "file://localhost/", 1), &dir).unwrap().path, Some(path));
        for invalid in ["file:///tmp/bad%2Fname.ts", "file:///tmp/bad%FF.ts", "file:///tmp/bad%00.ts", "file:///tmp/bad%G1.ts", "file:///tmp/bad%", "file:///tmp/bad%2"] {
            assert!(target_of(invalid, &dir).is_err(), "{invalid}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn urls_ports_fragments_and_remote_file_uris_are_kept_verbatim() {
        for uri in ["https://example.com:8443/code.ts#L42", "http://localhost:8080", "mailto:user@example.com", "urn:example:123", "file://other-host/tmp/code.ts:42"] {
            let target = target_of(uri, Path::new("/tmp")).unwrap();
            assert!(target.path.is_none());
            assert_eq!(target.argument, uri);
        }
    }

    #[test]
    fn existing_filenames_win_over_location_suffixes_and_invalid_positions_fail() {
        let (dir, path) = fixture("positions");
        let literal = dir.join("actual.ts:42");
        fs::write(&literal, "text").unwrap();
        let target = target_of(literal.to_str().unwrap(), &dir).unwrap();
        assert_eq!(target.path, Some(literal));
        assert_eq!(target.line, None);
        for invalid in ["", "line\nbreak", "null\0byte"] {
            assert!(target_of(invalid, &dir).is_err());
        }
        for suffix in [":0", ":1:0", ":9007199254740992"] {
            assert!(target_of(&format!("{}{suffix}", path.display()), &dir).is_err());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_working_directory_fails_before_opening_a_target() {
        let (dir, _) = fixture("cwd");
        let source = Target { session: "session".into(), block: "block".into() };
        let missing = dir.join("missing");
        let error = selection("https://example.com", &source, missing.to_str(), "/test/nvim").err().unwrap();
        assert_eq!(error, "open: working directory is not a directory");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn source_detection_excludes_binaries_ordinary_text_and_markdown() {
        for (name, mime, code) in [
            ("source.ts", "text/plain", true), ("Makefile", "text/plain", true),
            ("script", "text/x-shellscript", true), ("config.json", "application/json", true),
            ("icon.svg", "image/svg+xml", true), ("empty.rs", "inode/x-empty", true),
            ("source.ts", "video/mp2t", false), ("notes.md", "text/plain", false),
            ("manual.pdf", "application/pdf", false), ("script", "text/plain", false),
        ] {
            assert_eq!(is_code(Path::new(name), mime), code, "{name}: {mime}");
        }
    }
}
