//! Read-only charset resolution. No editor formatting settings are changed.
use super::FileEncoding;
use std::{fs, io::Read, path::Path};

const MAX_CONFIG_BYTES: usize = 65536;
const MAX_PATTERN_BYTES: usize = 512;
const MAX_ALTERNATIVES: usize = 64;
const MAX_WORK: usize = 16 * 1024 * 1024;

struct Budget(usize);
impl Budget {
    fn spend(&mut self, work: usize) -> Result<(), String> {
        self.0 = self.0.checked_sub(work).ok_or(
            "EditorConfig excede o orçamento de 16 Mi de operações; simplifique os padrões charset",
        )?;
        Ok(())
    }
}

fn config_text(path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    // Check before opening: opening a FIFO can block even before metadata/read.
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "{} deve ser arquivo regular sem link simbólico",
            path.display()
        ));
    }
    if metadata.len() > MAX_CONFIG_BYTES as u64 {
        return Err(format!("{} excede 64 KiB", path.display()));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Also protect the interval between checking the path and opening it.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err(format!("{} deve ser arquivo regular", path.display()));
    }
    let mut text = String::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if text.len() > MAX_CONFIG_BYTES {
        return Err(format!("{} excede 64 KiB", path.display()));
    }
    Ok(Some(text))
}

pub(super) fn resolve(
    path: &Path,
    _workspace: Option<&Path>,
) -> Result<Option<FileEncoding>, String> {
    let start = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    // EditorConfig inheritance belongs to the file: opening a smaller workspace
    // must not silently change its codec. Only root=true or the filesystem root stops it.
    let mut budget = Budget(MAX_WORK);
    for ancestor in start.ancestors() {
        let config = ancestor.join(".editorconfig");
        let Some(text) = config_text(&config)? else {
            continue;
        };
        budget.spend(text.len())?;
        let relative = path
            .strip_prefix(ancestor)
            .map_err(|error| error.to_string())?
            .to_string_lossy();
        #[cfg(windows)]
        let relative = relative.replace('\\', "/");
        let basename = path.file_name().unwrap_or_default().to_string_lossy();
        let mut pattern = None;
        let mut charset = None;
        let mut stop = false;
        let mut sections = 0;
        for line in text
            .strip_prefix('\u{feff}')
            .unwrap_or(&text)
            .lines()
            .map(str::trim)
        {
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            if line.starts_with('[') {
                let p = line
                    .strip_prefix('[')
                    .and_then(|section| section.strip_suffix(']'))
                    .ok_or_else(|| format!("{}: seção inválida", config.display()))?;
                sections += 1;
                if sections > 256 || p.len() > MAX_PATTERN_BYTES {
                    return Err("EditorConfig excede 256 seções ou padrão de 512 bytes".into());
                }
                pattern = Some(p);
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if pattern.is_none() && key.eq_ignore_ascii_case("root") {
                stop = value.eq_ignore_ascii_case("true");
            }
            if key.eq_ignore_ascii_case("charset")
                && let Some(pattern) = pattern
            {
                let candidate = if has_separator(pattern) {
                    relative.as_ref()
                } else {
                    basename.as_ref()
                };
                if matches_with_budget(
                    pattern.strip_prefix('/').unwrap_or(pattern),
                    candidate,
                    &mut budget,
                )
                .map_err(|error| format!("{}: {error}", config.display()))?
                {
                    charset = Some(value.to_ascii_lowercase());
                }
            }
        }
        if let Some(charset) = charset {
            return match charset.as_str() {
                "unset" => Ok(None),
                "utf-8" | "utf-8-bom" => Ok(Some(FileEncoding::Utf8)),
                "latin1" => Ok(Some(FileEncoding::Latin1)),
                "windows-1252" => Ok(Some(FileEncoding::Windows1252)),
                _ => Err(format!(
                    "{}: charset {charset} não suportado; selecione a codificação da pasta",
                    config.display()
                )),
            };
        }
        if stop {
            break;
        }
    }
    Ok(None)
}

fn has_separator(pattern: &str) -> bool {
    let mut class = false;
    let mut escaped = false;
    for character in pattern.chars() {
        if escaped {
            if character == '/' && !class {
                return true;
            }
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '[' if !class => class = true,
                ']' if class => class = false,
                '/' if !class => return true,
                _ => (),
            }
        }
    }
    false
}

/// Locate a brace expansion, ignoring escaped braces and braces in classes.
/// A singleton {word} is literal. Nested alternatives remain explicitly unsupported.
fn expansion(pattern: &str) -> Result<Option<(usize, usize, Vec<String>)>, String> {
    let mut chars = pattern.char_indices().peekable();
    let mut class = false;
    while let Some((start, character)) = chars.next() {
        match character {
            '\\' => {
                chars.next().ok_or("EditorConfig: escape incompleto")?;
            }
            '[' if !class => class = true,
            ']' if class => class = false,
            '{' if !class => {
                let mut end = None;
                let mut commas = Vec::new();
                let mut inner_class = false;
                while let Some((index, character)) = chars.next() {
                    match character {
                        '\\' => {
                            chars.next().ok_or("EditorConfig: escape incompleto")?;
                        }
                        '[' if !inner_class => inner_class = true,
                        ']' if inner_class => inner_class = false,
                        '{' if !inner_class => {
                            return Err(
                                "EditorConfig: alternativas aninhadas não suportadas".into()
                            );
                        }
                        ',' if !inner_class => commas.push(index),
                        '}' if !inner_class => {
                            end = Some(index);
                            break;
                        }
                        _ => (),
                    }
                }
                let end = end.ok_or("EditorConfig: chave sem fechamento")?;
                let inner = &pattern[start + 1..end];
                let choices = if !commas.is_empty() {
                    if commas.len() >= MAX_ALTERNATIVES {
                        return Err("EditorConfig: padrão excede 64 alternativas".into());
                    }
                    let mut choices = Vec::with_capacity(commas.len() + 1);
                    let mut from = start + 1;
                    for separator in commas.into_iter().chain(std::iter::once(end)) {
                        choices.push(pattern[from..separator].to_owned());
                        from = separator + 1;
                    }
                    choices
                } else if let Some((a, b)) = inner.split_once("..") {
                    let a: i32 = a
                        .parse()
                        .map_err(|_| "EditorConfig: intervalo numérico inválido")?;
                    let b: i32 = b
                        .parse()
                        .map_err(|_| "EditorConfig: intervalo numérico inválido")?;
                    if b <= a || i64::from(b) - i64::from(a) >= MAX_ALTERNATIVES as i64 {
                        return Err(
                            "EditorConfig: intervalo inválido ou excede 64 alternativas".into()
                        );
                    }
                    (a..=b).map(|number| number.to_string()).collect()
                } else {
                    continue;
                };
                return Ok(Some((start, end, choices)));
            }
            _ => (),
        }
    }
    Ok(None)
}

fn alternatives(pattern: &str, budget: &mut Budget) -> Result<Vec<String>, String> {
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err("EditorConfig: padrão excede 512 bytes".into());
    }
    let mut patterns = vec![pattern.to_owned()];
    loop {
        let mut expanded = false;
        let mut next = Vec::new();
        for pattern in patterns {
            budget.spend(pattern.len() + 1)?;
            if let Some((start, end, choices)) = expansion(&pattern)? {
                expanded = true;
                if next.len() + choices.len() > MAX_ALTERNATIVES {
                    return Err("EditorConfig: padrão excede 64 alternativas".into());
                }
                for choice in choices {
                    let value = format!("{}{}{}", &pattern[..start], choice, &pattern[end + 1..]);
                    if value.len() > MAX_PATTERN_BYTES {
                        return Err("EditorConfig: padrão expandido excede 512 bytes".into());
                    }
                    budget.spend(value.len() + 1)?;
                    next.push(value);
                }
            } else {
                if next.len() == MAX_ALTERNATIVES {
                    return Err("EditorConfig: padrão excede 64 alternativas".into());
                }
                next.push(pattern);
            }
        }
        if !expanded {
            return Ok(next);
        }
        patterns = next;
    }
}

#[derive(Clone)]
enum Token {
    Char(char),
    Any,
    Star(bool),
    GlobDirectories,
    Class(bool, Vec<(char, char)>),
}
fn class_character(chars: &[char], index: &mut usize) -> Result<char, String> {
    let character = *chars
        .get(*index)
        .ok_or("EditorConfig: classe sem fechamento")?;
    *index += 1;
    if character == '\\' {
        let escaped = *chars
            .get(*index)
            .ok_or("EditorConfig: escape incompleto na classe")?;
        *index += 1;
        Ok(escaped)
    } else {
        Ok(character)
    }
}
fn tokens(pattern: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<_> = pattern.chars().collect();
    let mut index = 0;
    let mut result = Vec::new();
    while index < chars.len() {
        match chars[index] {
            '*' if chars.get(index + 1) == Some(&'*') => {
                index += 1;
                if chars.get(index + 1) == Some(&'/') {
                    index += 1;
                    result.push(Token::GlobDirectories);
                } else {
                    result.push(Token::Star(true));
                }
            }
            '*' => result.push(Token::Star(false)),
            '?' => result.push(Token::Any),
            '\\' => {
                index += 1;
                result.push(Token::Char(
                    *chars.get(index).ok_or("EditorConfig: escape incompleto")?,
                ));
            }
            '[' => {
                index += 1;
                let negative = chars.get(index) == Some(&'!');
                if negative {
                    index += 1;
                }
                let mut ranges = Vec::new();
                while chars.get(index).is_some_and(|character| *character != ']') {
                    let a = class_character(&chars, &mut index)?;
                    let b = if chars.get(index) == Some(&'-')
                        && chars
                            .get(index + 1)
                            .is_some_and(|character| *character != ']')
                    {
                        index += 1;
                        class_character(&chars, &mut index)?
                    } else {
                        a
                    };
                    if b < a {
                        return Err("EditorConfig: intervalo de caracteres invertido".into());
                    }
                    if a == '/' || b == '/' {
                        return Err("EditorConfig: barra dentro de classe não suportada".into());
                    }
                    ranges.push((a, b));
                }
                if chars.get(index) != Some(&']') || ranges.is_empty() {
                    return Err("EditorConfig: classe de caracteres inválida".into());
                }
                result.push(Token::Class(negative, ranges));
            }
            character => result.push(Token::Char(character)),
        }
        index += 1;
    }
    Ok(result)
}
fn matches_with_budget(pattern: &str, text: &str, budget: &mut Budget) -> Result<bool, String> {
    budget.spend(text.len() + 1)?;
    let chars: Vec<_> = text.chars().collect();
    // Validate all alternatives before accepting one; malformed syntax cannot
    // silently disappear because an earlier alternative happened to match.
    let patterns = alternatives(pattern, budget)?
        .iter()
        .map(|pattern| tokens(pattern))
        .collect::<Result<Vec<_>, _>>()?;
    for tokens in patterns {
        let mut previous = vec![false; chars.len() + 1];
        previous[0] = true;
        let mut next = vec![false; chars.len() + 1];
        for token in tokens {
            let cost = match &token {
                Token::Class(_, ranges) => ranges.len().max(1),
                _ => 1,
            };
            budget.spend(
                (chars.len() + 1)
                    .checked_mul(cost)
                    .ok_or("EditorConfig: orçamento do padrão excedido")?,
            )?;
            next.fill(false);
            match token {
                Token::Star(cross) => {
                    next[0] = previous[0];
                    for j in 1..=chars.len() {
                        next[j] = previous[j] || (next[j - 1] && (cross || chars[j - 1] != '/'));
                    }
                }
                Token::GlobDirectories => {
                    let mut reachable = previous[0];
                    next[0] = previous[0];
                    for j in 1..=chars.len() {
                        reachable |= previous[j];
                        next[j] = previous[j] || (reachable && chars[j - 1] == '/');
                    }
                }
                _ => {
                    for j in 1..=chars.len() {
                        next[j] = previous[j - 1]
                            && match &token {
                                Token::Char(character) => chars[j - 1] == *character,
                                Token::Any => chars[j - 1] != '/',
                                Token::Class(negative, ranges) => {
                                    chars[j - 1] != '/'
                                        && (ranges
                                            .iter()
                                            .any(|(a, b)| *a <= chars[j - 1] && chars[j - 1] <= *b)
                                            != *negative)
                                }
                                _ => false,
                            };
                    }
                }
            }
            std::mem::swap(&mut previous, &mut next);
        }
        if previous[chars.len()] {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn matches(pattern: &str, text: &str) -> Result<bool, String> {
        matches_with_budget(pattern, text, &mut Budget(MAX_WORK))
    }
    #[test]
    fn editorconfig_patterns_match_paths_without_crossing_single_star() {
        for (pattern, path, expected) in [
            ("*.php", "ação.php", true),
            ("*.php", "src/a.php", false),
            ("**/*.php", "a.php", true),
            ("**/*.php", "src/deep/a.php", true),
            ("src/*.{php,inc}", "src/a.inc", true),
            ("[!a-c]?.php", "z1.php", true),
            ("[a-c]?.php", "z1.php", false),
            ("file{1..3}.php", "file2.php", true),
            ("file{-2..2}.php", "file-1.php", true),
        ] {
            assert_eq!(
                matches(pattern, path).unwrap(),
                expected,
                "{pattern} {path}"
            );
        }
        assert!(matches("{1..1000}", "2").is_err());
        assert!(matches("[abc", "a").is_err());
    }
    #[test]
    fn literal_braces_escaped_classes_and_later_alternatives_are_not_reinterpreted() {
        for (pattern, text, expected) in [
            ("{foo}.php", "{foo}.php", true),
            ("{foo}.php", "foo.php", false),
            (r"\{foo\}.php", "{foo}.php", true),
            ("[a{b,c}].php", "{.php", true),
            ("[a{b,c}].php", ",.php", true),
            (r"[\]ab].php", "].php", true),
            (r"[!\]ab].php", "].php", false),
            (r"{a\,b,c}.php", "a,b.php", true),
            ("{literal}{a,b}.php", "{literal}b.php", true),
            ("[a-c].php", "b.php", true),
            ("[-a].php", "-.php", true),
        ] {
            assert_eq!(
                matches(pattern, text).unwrap(),
                expected,
                "{pattern} {text}"
            );
        }
        for pattern in [
            "{a,{b,c}}",
            "{1..1}",
            "[z-a]",
            "{a,b",
            "trailing\\",
            "ab[e/]cd.i",
        ] {
            assert!(matches(pattern, "a").is_err(), "{pattern}");
        }
        assert!(!has_separator("ab[e/]cd.i"));
        assert!(has_separator("src/[e/]cd.i"));
    }
    #[test]
    fn glob_work_and_expansion_allocations_have_shared_limits() {
        assert!(matches(&"{a,b}".repeat(7), "aaaaaaa").is_err());
        assert!(matches(&"a".repeat(513), "a").is_err());
        let mut budget = Budget(100);
        assert!(matches_with_budget("*.php", "test.php", &mut budget).unwrap());
        assert!(matches_with_budget("*.php", "test.php", &mut budget).is_err());
        assert!(matches(&"a".repeat(500), &"a".repeat(40000)).is_err());
    }
    #[test]
    fn inherited_charsets_do_not_depend_on_the_opened_workspace_and_unset_stops_inheritance() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("service");
        fs::create_dir(&child).unwrap();
        fs::write(
            dir.path().join(".editorconfig"),
            "root=true\n[*]\ncharset=latin1\n",
        )
        .unwrap();
        fs::write(
            child.join(".editorconfig"),
            "root=false\n[*.txt]\ncharset=utf-8\n",
        )
        .unwrap();
        let php = child.join("a.php");
        assert_eq!(
            resolve(&php, Some(&child)).unwrap(),
            Some(FileEncoding::Latin1)
        );
        assert_eq!(
            resolve(&php, Some(dir.path())).unwrap(),
            Some(FileEncoding::Latin1)
        );
        assert_eq!(
            resolve(&child.join("a.txt"), Some(&child)).unwrap(),
            Some(FileEncoding::Utf8)
        );
        fs::write(
            child.join(".editorconfig"),
            "[*.php]\ncharset=latin1\n[a.php]\ncharset=unset\n",
        )
        .unwrap();
        assert_eq!(resolve(&php, Some(&child)).unwrap(), None);
        fs::write(
            child.join(".editorconfig"),
            "\u{feff}root=true\n[*.txt]\ncharset=utf-8\n",
        )
        .unwrap();
        assert_eq!(resolve(&php, Some(&child)).unwrap(), None);
    }
    #[test]
    fn config_limits_and_unsupported_charset_fail_instead_of_falling_back() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".editorconfig");
        let php = dir.path().join("a.php");
        fs::write(&config, "root=true\n[*.php]\ncharset=utf-16le\n").unwrap();
        assert!(
            resolve(&php, Some(dir.path()))
                .unwrap_err()
                .contains("não suportado")
        );
        fs::write(&config, "a".repeat(MAX_CONFIG_BYTES + 1)).unwrap();
        assert!(
            resolve(&php, Some(dir.path()))
                .unwrap_err()
                .contains("64 KiB")
        );
        fs::write(&config, "[*]\ncharset=utf-8\n".repeat(257)).unwrap();
        assert!(
            resolve(&php, Some(dir.path()))
                .unwrap_err()
                .contains("256 seções")
        );
    }
    #[test]
    #[cfg(unix)]
    fn config_links_and_fifos_are_rejected_without_opening_a_blocking_stream() {
        use std::{
            ffi::CString,
            os::unix::{ffi::OsStrExt, fs::symlink},
            time::Duration,
        };
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".editorconfig");
        let php = dir.path().join("a.php");
        let own = dir.path().join("own-config");
        fs::write(&own, "root=true\n[*]\ncharset=latin1\n").unwrap();
        symlink(&own, &config).unwrap();
        assert!(
            resolve(&php, Some(dir.path()))
                .unwrap_err()
                .contains("link simbólico")
        );
        fs::remove_file(&config).unwrap();
        let name = CString::new(config.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a NUL-terminated fixture path; no pointers are retained.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender.send(resolve(&php, None)).unwrap();
        });
        let error = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("FIFO must not block opening the workspace")
            .unwrap_err();
        assert!(error.contains("arquivo regular"));
        worker.join().unwrap();
    }
}
