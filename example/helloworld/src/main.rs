use std::collections::HashMap;

fn detect_locale() -> &'static str {
    let lang = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default();
    if !lang.is_empty() {
        if lang.to_ascii_lowercase().starts_with("de") {
            return "de_de";
        }
        return "en_us";
    }
    if let Ok(content) = std::fs::read_to_string("/etc/locale.conf") {
        if let Some(line) = content.lines().find(|l| l.starts_with("LANG=")) {
            let lang = line.trim_start_matches("LANG=");
            if lang.to_ascii_lowercase().starts_with("de") {
                return "de_de";
            }
        }
    }
    "en_us"
}

fn load_strings(locale: &str) -> HashMap<String, String> {
    let candidates = [
        format!("/usr/share/helloworld/lang/{}.json", locale),
        format!("lang/{}.json", locale),
    ];
    for path in candidates {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&content) {
                return map;
            }
        }
    }
    let mut fallback = HashMap::new();
    fallback.insert("hello".to_string(), "Hello, world!".to_string());
    fallback
}

fn main() {
    let strings = load_strings(detect_locale());
    let hello = strings
        .get("hello")
        .cloned()
        .unwrap_or_else(|| "Hello, world!".to_string());
    println!("{}", hello);
}