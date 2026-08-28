use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;

use github_actions_local_cache::config::{
    CacheContext, parse_boolean, parse_lines, validate_key, validate_patterns,
};
use github_actions_local_cache::digest::sha256;
use github_actions_local_cache::{
    CacheError, RestoreRequest, SaveRequest, restore_cache, save_cache,
};
use serde_json::json;

fn input(name: &str) -> String {
    let canonical = format!("INPUT_{}", name.to_ascii_uppercase());
    env::var(&canonical)
        .or_else(|_| env::var(canonical.replace('-', "_")))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn required_input(name: &str) -> Result<String, CacheError> {
    let value = input(name);
    if value.is_empty() {
        return Err(CacheError::new(
            "invalid-input",
            format!("{name} is required"),
        ));
    }
    Ok(value)
}

fn set_output(name: &str, value: &str) -> Result<(), CacheError> {
    let path = env::var("GITHUB_OUTPUT")
        .map_err(|_| CacheError::new("invalid-environment", "GITHUB_OUTPUT is required"))?;
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| CacheError::io("output-write", error))?;
    writeln!(file, "{name}={value}").map_err(|error| CacheError::io("output-write", error))
}

fn event(value: serde_json::Value) {
    println!("{value}");
}

fn run_restore() -> Result<(), CacheError> {
    let started = Instant::now();
    let key = required_input("key")?;
    validate_key(&key)?;
    let patterns = parse_lines(&required_input("path")?);
    validate_patterns(&patterns)?;
    let restore_keys = parse_lines(&input("restore-keys"));
    for prefix in &restore_keys {
        validate_key(prefix)?;
    }
    let cache_dir = input("cache-dir");
    let context =
        CacheContext::from_environment((!cache_dir.is_empty()).then_some(cache_dir.as_str()))?;
    let requested_digest = sha256(&key);
    let result = restore_cache(RestoreRequest {
        context,
        key,
        restore_keys,
    })?;
    set_output(
        "cache-hit",
        if result.cache_match.as_str() == "exact" {
            "true"
        } else {
            "false"
        },
    )?;
    set_output("cache-match", result.cache_match.as_str())?;
    event(
        json!({ "event": "local-cache.restore", "match": result.cache_match.as_str(), "digest": result.digest.unwrap_or(requested_digest), "files": result.files, "bytes": result.bytes, "elapsedMs": started.elapsed().as_millis() }),
    );
    Ok(())
}

fn run_save() -> Result<(), CacheError> {
    let started = Instant::now();
    let key = required_input("key")?;
    validate_key(&key)?;
    let patterns = parse_lines(&required_input("path")?);
    validate_patterns(&patterns)?;
    let cache_dir = input("cache-dir");
    let context =
        CacheContext::from_environment((!cache_dir.is_empty()).then_some(cache_dir.as_str()))?;
    let digest = sha256(&key);
    let result = save_cache(SaveRequest {
        context,
        key,
        patterns,
    })?;
    set_output("cache-save", result.as_str())?;
    event(
        json!({ "event": "local-cache.save", "result": result.as_str(), "digest": digest, "elapsedMs": started.elapsed().as_millis() }),
    );
    Ok(())
}

fn main() {
    let operation = env::args().nth(1).unwrap_or_default();
    let fail_input = input("fail-on-cache-error");
    let fail_on_error = parse_boolean(
        if fail_input.is_empty() {
            "true"
        } else {
            &fail_input
        },
        "fail-on-cache-error",
    );
    let fail_on_error = match fail_on_error {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "::error::{}",
                json!({"event":"local-cache.error","code":error.code})
            );
            std::process::exit(1);
        }
    };
    let result = match operation.as_str() {
        "restore" => run_restore(),
        "save" => run_save(),
        _ => Err(CacheError::new(
            "invalid-operation",
            "expected restore or save",
        )),
    };
    if let Err(error) = result {
        let digest = input("key");
        let digest = validate_key(&digest).is_ok().then(|| sha256(digest));
        let message = json!({ "event": "local-cache.error", "code": error.code, "digest": digest });
        if fail_on_error {
            eprintln!("::error::{message}");
            std::process::exit(1);
        }
        eprintln!("::warning::{message}");
        let output = if operation == "restore" {
            set_output("cache-hit", "false").and_then(|_| set_output("cache-match", "error"))
        } else {
            set_output("cache-save", "error")
        };
        if output.is_err() {
            std::process::exit(1);
        }
    }
}
