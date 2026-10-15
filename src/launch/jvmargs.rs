//! The parts of building the game's Java command line that are plain decisions: splitting what the player typed
//! into arguments, which tuning flags to give Java, and when a command line is too long for Windows. A port of the
//! Java edition's `JvmArguments`, so both editions start the game the same way.

use crate::config::DEFAULT_JAVA_ARGS;

/// Windows refuses to start a program whose command line is longer than 32767 characters. Big modpacks have
/// hundreds of libraries on the class path and can go over it, so stay well clear.
pub const WINDOWS_COMMAND_LINE_BUDGET: usize = 30000;

/// Splits Java arguments as typed by a person into separate arguments. Spaces separate arguments, except inside
/// double quotes, so `-Dname="two words"` stays one argument (without the quotes). Only double quotes group, since
/// apostrophes turn up in names and paths (`C:\Users\O'Brien`).
pub fn split(arguments: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut in_quote = false;
    for c in arguments.chars() {
        if in_quote {
            if c == '"' {
                in_quote = false;
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quote = true;
            in_token = true;
        } else if c.is_whitespace() {
            if in_token {
                result.push(std::mem::take(&mut current));
                in_token = false;
            }
        } else {
            current.push(c);
            in_token = true;
        }
    }
    if in_token {
        result.push(current);
    }
    result
}

/// If these are the person's own Java arguments, as opposed to nothing or the ones BlockForge ships with. Their own
/// arguments are always used exactly as they are.
pub fn is_custom(arguments: Option<&str>) -> bool {
    match arguments {
        None => false,
        Some(a) if a.trim().is_empty() => false,
        Some(a) => split(a) != split(DEFAULT_JAVA_ARGS),
    }
}

/// The garbage collector settings to start the game with when the person hasn't set their own Java arguments:
/// the flags Mojang's own launcher uses, which work on every Java from 8 to the newest, with the region size fitted
/// to the amount of memory instead of always 32 MB. Nothing here costs anything on a small computer: no fixed
/// starting heap, no thread counts, and smaller regions for small heaps.
pub fn performance_flags(maximum_heap_mb: u32, processors: usize) -> Vec<String> {
    performance_flags_for(maximum_heap_mb, processors, None)
}

/// [`performance_flags`] plus the settings that make the game open sooner, for the Java in use.
///
/// Skipping the bytecode check of the game's and mods' classes (`-XX:-BytecodeVerificationRemote`) saves about
/// 5% of the time from pressing Play to the title screen on Java 17 to 21 (measured on Minecraft 1.21.1). On Java 25
/// (Minecraft 26.x) it made no difference, so it is left out there. Java accepts it as a diagnostic option;
/// the check only ever guards against broken classes, which then fail with a different error instead of a verify
/// error.
pub fn performance_flags_for(maximum_heap_mb: u32, processors: usize, java_major: Option<u32>) -> Vec<String> {
    let mut flags = Vec::new();
    if java_major.is_some_and(|m| (17..=21).contains(&m)) {
        flags.push("-XX:+UnlockDiagnosticVMOptions".to_string());
        flags.push("-XX:-BytecodeVerificationRemote".to_string());
    }
    // with a single processor G1's background work competes with the game itself; Java then picks the Serial
    // collector on its own, which is the better choice there
    if processors <= 1 {
        return flags;
    }
    flags.push("-XX:+UnlockExperimentalVMOptions".to_string());
    flags.push("-XX:+UseG1GC".to_string());
    flags.push("-XX:G1NewSizePercent=20".to_string());
    flags.push("-XX:G1ReservePercent=20".to_string());
    flags.push("-XX:MaxGCPauseMillis=50".to_string());
    let region = g1_region_size_mb(maximum_heap_mb);
    if region > 0 {
        flags.push(format!("-XX:G1HeapRegionSize={region}M"));
    }
    flags
}

/// Largest power of two no bigger than heap/64 and no bigger than 32, or 0 to let Java choose.
pub fn g1_region_size_mb(maximum_heap_mb: u32) -> u32 {
    let target = maximum_heap_mb / 64;
    if target < 1 {
        return 0;
    }
    let mut size = 1;
    while size * 2 <= target && size < 32 {
        size *= 2;
    }
    size
}

/// The name of a -XX option, -D property or similar, so the same setting isn't given twice with two values.
pub fn option_name(argument: &str) -> String {
    if argument.starts_with("-XX:+") || argument.starts_with("-XX:-") {
        return format!("-XX:{}", &argument[5..]);
    }
    match argument.find('=') {
        Some(i) => argument[..i].to_string(),
        None => argument.to_string(),
    }
}

/// Adds the flags whose setting isn't already in the arguments.
pub fn add_missing(arguments: &mut Vec<String>, flags: &[String]) {
    let mut present: Vec<String> = arguments.iter().map(|a| option_name(a)).collect();
    let is_collector = |a: &str| a.starts_with("-XX:+Use") && a.ends_with("GC");
    for flag in flags {
        // only one collector may be chosen
        if is_collector(flag) && arguments.iter().any(|a| is_collector(a)) {
            continue;
        }
        let name = option_name(flag);
        if !present.contains(&name) {
            arguments.push(flag.clone());
            present.push(name);
        }
    }
}

/// How long Windows will see the command line as: every argument, quoted when needed, with spaces between.
pub fn windows_command_line_length(arguments: &[String]) -> usize {
    arguments
        .iter()
        .map(|a| a.chars().count() + 1 + if a.is_empty() || a.contains(' ') || a.contains('\t') { 2 } else { 0 })
        .sum()
}

/// If the Java options should go into an argument file (`java @file`) because the command line would be too long
/// for Windows. Argument files only exist since Java 9; on Java 8 there is nothing better to do than try.
pub fn needs_argument_file(arguments: &[String], windows: bool, java_major: Option<u32>) -> bool {
    windows
        && java_major.is_some_and(|m| m >= 9)
        && windows_command_line_length(arguments) > WINDOWS_COMMAND_LINE_BUDGET
}

/// Arguments in the format of a Java argument file: one per line, quoted, with backslashes and quotes escaped as
/// Java expects inside quotes.
pub fn to_argument_file(arguments: &[String]) -> String {
    let mut out = String::new();
    for a in arguments {
        out.push('"');
        out.push_str(&a.replace('\\', "\\\\").replace('"', "\\\""));
        out.push_str("\"\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_follows_the_java_edition() {
        assert_eq!(split(r#"-Xss1M -Dname="two words" -Dx="""#), vec!["-Xss1M", "-Dname=two words", "-Dx="]);
        // apostrophes do not group
        assert_eq!(split("-Dp=C:\\Users\\O'Brien -Xmx1G"), vec!["-Dp=C:\\Users\\O'Brien", "-Xmx1G"]);
        assert_eq!(split("  a   b\tc "), vec!["a", "b", "c"]);
        assert!(split("").is_empty());
        // an empty pair of quotes is still an argument
        assert_eq!(split("\"\""), vec![""]);
    }

    #[test]
    fn default_arguments_are_not_custom() {
        assert!(!is_custom(None));
        assert!(!is_custom(Some("   ")));
        assert!(!is_custom(Some(DEFAULT_JAVA_ARGS)));
        assert!(!is_custom(Some(&format!("  {DEFAULT_JAVA_ARGS}  "))));
        assert!(is_custom(Some("-Xmn512M")));
    }

    #[test]
    fn region_size_fits_the_heap() {
        assert_eq!(g1_region_size_mb(0), 0);
        assert_eq!(g1_region_size_mb(2048), 32);
        assert_eq!(g1_region_size_mb(2047), 16);
        assert_eq!(g1_region_size_mb(512), 8);
        assert_eq!(g1_region_size_mb(64), 1);
        assert_eq!(g1_region_size_mb(63), 0);
        assert_eq!(g1_region_size_mb(16384), 32);
    }

    #[test]
    fn fast_start_flags_depend_on_the_java_version() {
        let new = performance_flags_for(4096, 8, Some(21));
        assert!(new.contains(&"-XX:+UnlockDiagnosticVMOptions".to_string()));
        // the unlock has to come before the option that needs it
        let unlock = new.iter().position(|f| f == "-XX:+UnlockDiagnosticVMOptions").unwrap();
        let skip = new.iter().position(|f| f == "-XX:-BytecodeVerificationRemote").unwrap();
        assert!(unlock < skip);
        assert!(!performance_flags_for(4096, 8, Some(8)).iter().any(|f| f.contains("BytecodeVerification")));
        assert!(!performance_flags_for(4096, 8, Some(25)).iter().any(|f| f.contains("BytecodeVerification")));
        assert!(!performance_flags_for(4096, 8, None).iter().any(|f| f.contains("BytecodeVerification")));
        // a single-core computer still gets the faster start, just not the G1 settings
        let one = performance_flags_for(4096, 1, Some(21));
        assert!(one.iter().any(|f| f.contains("BytecodeVerificationRemote")) && !one.iter().any(|f| f.contains("G1")));
    }

    #[test]
    fn performance_flags_skip_single_core() {
        assert!(performance_flags(4096, 1).is_empty());
        let flags = performance_flags(4096, 8);
        assert!(flags.contains(&"-XX:+UseG1GC".to_string()));
        assert!(flags.contains(&"-XX:G1HeapRegionSize=32M".to_string()));
    }

    #[test]
    fn missing_flags_are_added_once() {
        let mut args = vec!["-Xmx2G".to_string(), "-XX:+UseZGC".to_string(), "-XX:MaxGCPauseMillis=20".to_string()];
        add_missing(&mut args, &performance_flags(2048, 4));
        // another collector was chosen, and the pause target was already set
        assert!(!args.contains(&"-XX:+UseG1GC".to_string()));
        assert_eq!(args.iter().filter(|a| a.starts_with("-XX:MaxGCPauseMillis")).count(), 1);
        assert!(args.contains(&"-XX:G1ReservePercent=20".to_string()));
        assert_eq!(option_name("-XX:-UseG1GC"), "-XX:UseG1GC");
        assert_eq!(option_name("-Dfoo=bar"), "-Dfoo");
    }

    #[test]
    fn long_command_lines_move_to_a_file() {
        let long = vec!["x".repeat(31000)];
        assert!(needs_argument_file(&long, true, Some(17)));
        assert!(!needs_argument_file(&long, true, Some(8)));
        assert!(!needs_argument_file(&long, true, None));
        assert!(!needs_argument_file(&long, false, Some(21)));
        assert_eq!(windows_command_line_length(&["a b".to_string(), String::new()]), 9);
        assert_eq!(
            to_argument_file(&["a\\b".to_string(), "say \"hi\"".to_string()]),
            "\"a\\\\b\"\n\"say \\\"hi\\\"\"\n"
        );
    }
}
