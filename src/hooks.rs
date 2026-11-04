//! Completion hooks use argv, never a shell. Double quotes group arguments;
//! backslashes are literal unless followed by a quote: pairs become one
//! backslash, and an odd remaining backslash escapes the quote.

#[derive(Debug, PartialEq, Eq)]
pub struct HookSpec {
    pub executable: String,
    pub args: Vec<String>,
}

impl HookSpec {
    pub fn parse(template: &str) -> Result<Self, String> {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quoted = false;
        let mut started = false;
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    let mut count = 1;
                    while chars.peek() == Some(&'\\') {
                        chars.next();
                        count += 1;
                    }
                    if chars.peek() == Some(&'"') {
                        for _ in 0..count / 2 {
                            word.push('\\');
                        }
                        if count % 2 == 1 {
                            chars.next();
                            word.push('"');
                        }
                    } else {
                        for _ in 0..count {
                            word.push('\\');
                        }
                    }
                    started = true;
                }
                '"' => {
                    quoted = !quoted;
                    started = true;
                }
                c if c.is_whitespace() && !quoted => {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                '&' | '|' | ';' | '<' | '>' if !quoted => {
                    return Err("Completion hooks do not support shell syntax; use an executable and quoted arguments.".into());
                }
                _ => {
                    word.push(c);
                    started = true;
                }
            }
        }
        if quoted {
            return Err("Completion hook has an unmatched double quote.".into());
        }
        if started {
            words.push(word);
        }
        if words.is_empty() || words[0].is_empty() {
            return Err("Completion hook needs a literal executable.".into());
        }
        let executable = words.remove(0);
        if executable.contains(['{', '}']) {
            return Err("Completion hook placeholders are allowed only in arguments.".into());
        }
        let name = executable
            .rsplit(['/', '\\'])
            .next()
            .unwrap()
            .trim_end_matches(['.', ' '])
            .to_ascii_lowercase();
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        if matches!(
            stem,
            "cmd"
                | "powershell"
                | "pwsh"
                | "sh"
                | "bash"
                | "dash"
                | "zsh"
                | "wscript"
                | "cscript"
                | "mshta"
                | "rundll32"
                | "python"
                | "python3"
                | "perl"
                | "ruby"
                | "node"
                | "wsl"
                | "busybox"
                | "env"
                | "fish"
                | "ksh"
                | "tclsh"
                | "lua"
                | "php"
                | "deno"
                | "bun"
        ) || stem.starts_with("python")
            || name
                .rsplit_once('.')
                .is_some_and(|(_, extension)| extension != "exe")
        {
            return Err("Completion hooks cannot run scripts or interpreters; choose a native executable, for example explorer.exe \"{dir}\".".into());
        }
        Ok(Self {
            executable,
            args: words,
        })
    }

    /// Replace each template placeholder once. Values are never parsed again.
    pub fn expand(mut self, file: &str, dir: &str, url: &str, title: &str) -> Self {
        for arg in &mut self.args {
            let mut expanded = String::new();
            let mut rest = arg.as_str();
            while !rest.is_empty() {
                if let Some((key, value)) = [
                    ("{file}", file),
                    ("{dir}", dir),
                    ("{url}", url),
                    ("{title}", title),
                ]
                .into_iter()
                .find(|(key, _)| rest.starts_with(key))
                {
                    expanded.push_str(value);
                    rest = &rest[key.len()..];
                } else {
                    let c = rest.chars().next().unwrap();
                    expanded.push(c);
                    rest = &rest[c.len_utf8()..];
                }
            }
            *arg = expanded;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_is_single_pass_and_preserves_argv() {
        let hook = HookSpec::parse(r#"receiver.exe "{file}" "" "{title}" "{url}""#)
            .unwrap()
            .expand(
                r#"C:\a & b\"{title}".mp4"#,
                "unused",
                "https://example/a|b%20",
                "雪 {url}",
            );
        assert_eq!(
            hook.args,
            [
                r#"C:\a & b\"{title}".mp4"#,
                "",
                "雪 {url}",
                "https://example/a|b%20"
            ]
        );
    }

    #[test]
    fn explorer_and_windows_paths_are_preserved() {
        let hook = HookSpec::parse(r#""C:\Program Files\viewer.exe" "{dir}""#).unwrap();
        assert_eq!(hook.executable, r"C:\Program Files\viewer.exe");
        assert_eq!(
            HookSpec::parse(r#"explorer.exe "{dir}""#)
                .unwrap()
                .expand("", r"C:\Videos", "", "")
                .args,
            [r"C:\Videos"]
        );
    }

    #[test]
    fn rejects_shells_scripts_and_invalid_templates() {
        for template in [
            "",
            "{file} x",
            "viewer.exe \"bad",
            "cmd.exe /c {file}",
            "pwsh -Command {title}",
            "a.cmd {file}",
            "a.cmd. {file}",
            "a.py {file}",
            "python3.14.exe {file}",
            "viewer.exe x & sentinel.exe",
            "wscript.exe x",
        ] {
            assert!(HookSpec::parse(template).is_err(), "{template}");
        }
    }
}
