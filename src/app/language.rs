use std::path::Path;

/// GPUI's Syntect backend accepts extension tokens directly; don't restrict its grammar set.
pub fn detect(path: &Path, text: &str) -> String {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    match name {
        "Dockerfile" | "Containerfile" => return "Dockerfile".into(),
        "Makefile" | "GNUmakefile" => return "Makefile".into(),
        "CMakeLists.txt" => return "cmake".into(),
        "Gemfile" | "Rakefile" => return "rb".into(),
        ".bashrc" | ".bash_profile" | ".zshrc" | ".profile" => return "sh".into(),
        _ => {}
    }
    if let Some(extension) = path.extension().and_then(|s| s.to_str()) {
        return extension.to_owned();
    }
    if let Some(shebang) = text.lines().next().and_then(|s| s.strip_prefix("#!")) {
        for word in shebang.split_whitespace() {
            let command = word.rsplit('/').next().unwrap_or(word);
            match command {
                "python" | "python2" | "python3" => return "py".into(),
                "node" | "nodejs" => return "js".into(),
                "ruby" => return "rb".into(),
                "bash" | "sh" | "zsh" | "fish" => return command.into(),
                "perl" => return "pl".into(),
                "pwsh" => return "ps1".into(),
                _ => {}
            }
        }
    }
    "text".into()
}

#[cfg(test)]
mod tests {
    use super::detect;
    use std::path::Path;
    #[test]
    fn additional_languages_resolve_to_real_bundled_grammars() {
        let syntaxes = two_face::syntax::extra_newlines();
        for path in [
            "code.go",
            "Code.java",
            "code.cpp",
            "code.cs",
            "code.rb",
            "code.php",
            "code.swift",
            "code.kt",
            "code.lua",
            "code.sql",
            "code.tsx",
            "Dockerfile",
            "Makefile",
            "CMakeLists.txt",
        ] {
            let language = detect(Path::new(path), "");
            let syntax = syntaxes
                .find_syntax_by_token(&language)
                .unwrap_or_else(|| panic!("No bundled grammar for {path}: {language}"));
            assert_ne!(syntax.name, "Plain Text");
        }
    }
    #[test]
    fn filenames_and_shebangs_do_not_override_an_explicit_extension() {
        assert_eq!(
            detect(
                Path::new("script"),
                "#!/usr/bin/env -S python3 -u\nprint(1)"
            ),
            "py"
        );
        assert_eq!(detect(Path::new("script"), "#!/bin/bash\ntrue"), "bash");
        assert_eq!(detect(Path::new("Gemfile"), ""), "rb");
        assert_eq!(detect(Path::new("code.rs"), "#!/bin/sh"), "rs");
        assert_eq!(detect(Path::new("README"), "plain"), "text");
    }
}
