use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let prompts_dir = manifest_dir.join("prompts");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let generated = out_dir.join("embedded_prompts.rs");

    println!("cargo:rerun-if-changed={}", prompts_dir.display());

    let mut entries = Vec::new();
    for language_dir in fs::read_dir(&prompts_dir).unwrap() {
        let language_dir = language_dir.unwrap();
        if !language_dir.file_type().unwrap().is_dir() {
            continue;
        }
        let language = language_dir.file_name().to_string_lossy().to_string();
        collect_prompt_files(
            &language,
            &language_dir.path(),
            &language_dir.path(),
            &mut entries,
        );
    }
    entries.sort();

    let mut output = String::from(
        "pub(crate) fn embedded_template(language: &str, name: &str) -> Option<&'static str> {\n",
    );
    output.push_str("    match (language, name) {\n");
    for (language, name, path) in entries {
        println!("cargo:rerun-if-changed={}", path.display());
        output.push_str(&format!(
            "        ({:?}, {:?}) => Some(include_str!(r#\"{}\"#)),\n",
            language,
            name,
            path.display()
        ));
    }
    output.push_str("        _ => None,\n");
    output.push_str("    }\n");
    output.push_str("}\n");

    fs::write(generated, output).unwrap();
}

fn collect_prompt_files(
    language: &str,
    base: &Path,
    dir: &Path,
    entries: &mut Vec<(String, String, PathBuf)>,
) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_prompt_files(language, base, &path, entries);
            continue;
        }
        let relative = path
            .strip_prefix(base)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        entries.push((language.to_string(), relative, path));
    }
}
