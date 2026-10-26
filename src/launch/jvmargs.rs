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

/// A Java memory size (`512m`, `4G`, `1048576k`, `2147483648`) in MB, as `-Xmx`/`-Xms` take it.
pub fn memory_mb(value: &str) -> Option<u64> {
    let value = value.trim();
    let (digits, unit) = match value.char_indices().last()? {
        (i, c) if c.is_ascii_alphabetic() => (&value[..i], c.to_ascii_lowercase()),
        _ => (value, 'b'),
    };
    let n: u64 = digits.parse().ok()?;
    Some(match unit {
        'b' => n / (1024 * 1024),
        'k' => n / 1024,
        'm' => n,
        'g' => n.saturating_mul(1024),
        't' => n.saturating_mul(1024 * 1024),
        _ => return None,
    })
}

/// The person's own Java arguments with a starting heap (`-Xms`) that is bigger than the most the game may use taken
/// out: Java refuses to start at all then ("Initial heap size set to a larger value than the maximum heap size").
/// `max_mb` is what the launcher gives the game; a `-Xmx` among the arguments wins over it, as it does in Java.
/// Returns a note for the log when something was taken out.
pub fn drop_oversized_initial_heap(args: &mut Vec<String>, max_mb: u32) -> Option<String> {
    let own_max = args.iter().rev().find_map(|a| a.strip_prefix("-Xmx").and_then(memory_mb));
    let max = own_max.unwrap_or(max_mb as u64);
    let too_big = |a: &String| a.strip_prefix("-Xms").and_then(memory_mb).is_some_and(|ms| ms > max);
    let dropped: Vec<String> = args.iter().filter(|a| too_big(a)).cloned().collect();
    if dropped.is_empty() {
        return None;
    }
    args.retain(|a| !too_big(a));
    Some(format!(
        "Left out {} from the Java arguments: the game may only use {max} MB, and Java won't start when it is told to begin with more",
        dropped.join(" ")
    ))
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
/// to the amount of memory instead of always 32 MB, the same on any number of processors. Nothing here costs anything
/// on a small computer: no fixed starting heap (`-Xms` as big as `-Xmx` took 700 MB more memory for no gain), no
/// pre-touched memory, no thread counts, and smaller regions for small heaps. Measured against Parallel, Serial, ZGC
/// and Shenandoah on 2 and 4 processors (26.3, 2 GB): G1 with these flags had the fewest long frames; ZGC and
/// Shenandoah used 1 GB more memory for the same heap and loaded worlds slower.
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
pub fn performance_flags_for(maximum_heap_mb: u32, _processors: usize, java_major: Option<u32>) -> Vec<String> {
    let mut flags = Vec::new();
    if java_major.is_some_and(|m| (17..=21).contains(&m)) {
        flags.push("-XX:+UnlockDiagnosticVMOptions".to_string());
        flags.push("-XX:-BytecodeVerificationRemote".to_string());
    }
    // G1 on every computer, also one with a single processor: Java would pick the Serial collector there by itself,
    // which froze the game for 0.3-1.3 s while a world loaded; G1's longest pause was about 0.2 s (measured, 26.3)
    flags.push("-XX:+UnlockExperimentalVMOptions".to_string());
    flags.push("-XX:+UseG1GC".to_string());
    flags.push("-XX:G1NewSizePercent=20".to_string());
    flags.push("-XX:G1ReservePercent=20".to_string());
    flags.push("-XX:MaxGCPauseMillis=50".to_string());
    let region = g1_region_size_mb(maximum_heap_mb);
    if region > 0 {
        flags.push(format!("-XX:G1HeapRegionSize={region}M"));
    }
    // Minecraft 1.8.9 and 1.12.2 ask for a full collection (System.gc()) every time a world is opened or left; with
    // this G1 does it alongside the game, and the stop went from 264-436 ms to 36-67 ms. Newer versions only ask when
    // memory runs out. Every Java from 8 to 25 knows the option.
    flags.push("-XX:+ExplicitGCInvokesConcurrent".to_string());
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

/// How the game's start-up is tuned (the `startupMode` setting).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartupMode {
    /// Pick from what this computer has.
    Auto,
    /// Always the quick compiler only.
    Fast,
    /// Never touch the compiler: full speed once the game is warm.
    Full,
}

impl StartupMode {
    /// From the setting's text; anything unknown (or an old settings file without it) is automatic.
    pub fn parse(text: &str) -> StartupMode {
        match text.trim().to_ascii_lowercase().as_str() {
            "fast" => StartupMode::Fast,
            "full" => StartupMode::Full,
            _ => StartupMode::Auto,
        }
    }
}

/// With this many logical processors or fewer, Java's optimizing compiler (C2) takes the processors the game's own
/// start needs, and only the quick compiler (C1) is used. Measured (Minecraft 1.21.1 Fabric, Java 21, seconds to the
/// title screen, tiered / quick compiler only): 1 core 30 / 12.7, 2 cores 16-20 / 9.5-10, 3 cores 10.2 / 8.0,
/// 4 cores 11.4 / 7.5, 6 cores 11.9 / 8.6, 8 cores 8.4 / 8.6 (no difference). Limiting the compiler's threads
/// (`-XX:CICompilerCount=2`) gave nothing at any size, so it is not used.
pub const QUICK_COMPILER_MAX_PROCESSORS: usize = 6;
/// Up to this many, a computer with little free memory still takes the quick compiler: compiled code and the
/// compiler's own work compete with the game for it.
pub const SHORT_OF_MEMORY_MAX_PROCESSORS: usize = 8;
/// "Little free memory" for the rule above (MB).
pub const LOW_FREE_MEMORY_MB: i64 = 3072;

/// What start-up flags were chosen, and the line that says so in the game's log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Startup {
    pub flags: Vec<String>,
    pub note: String,
}

/// Chooses the compiler settings for the start from what the computer has. With few processors the optimizing
/// compiler's threads compete with the game's own start and the quick compiler alone gets to the title screen up to
/// three times sooner (see the constants). The price is speed in long sessions: a CPU-bound test ran at half the speed
/// with only the quick compiler (about 71,000 against 36,000 operations a second on two cores), so computers with
/// processors to spare keep the optimizing compiler, and `Full performance` is there for slow computers whose players
/// care more about frame rates in long sessions.
///
/// `available_mb` is the free memory, 0 when unknown.
pub fn startup_flags(mode: StartupMode, processors: usize, available_mb: i64) -> Startup {
    let quick = || vec!["-XX:TieredStopAtLevel=1".to_string()];
    match mode {
        StartupMode::Full => {
            Startup { flags: Vec::new(), note: "Start-up: full performance (the compiler is left alone)".to_string() }
        }
        StartupMode::Fast => Startup {
            flags: quick(),
            note: format!("Start-up: quick compiler (chosen in the settings; {processors} processor cores)"),
        },
        StartupMode::Auto => {
            let short_of_memory = available_mb > 0 && available_mb < LOW_FREE_MEMORY_MB;
            if processors <= QUICK_COMPILER_MAX_PROCESSORS {
                Startup { flags: quick(), note: format!("Start-up: quick compiler ({processors} cores)") }
            } else if processors <= SHORT_OF_MEMORY_MAX_PROCESSORS && short_of_memory {
                Startup {
                    flags: quick(),
                    note: format!("Start-up: quick compiler ({processors} cores, {available_mb} MB of memory free)"),
                }
            } else {
                Startup { flags: Vec::new(), note: format!("Start-up: full performance ({processors} cores)") }
            }
        }
    }
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
        // a single-core computer gets the faster start and the G1 settings too
        let one = performance_flags_for(4096, 1, Some(21));
        assert!(one.iter().any(|f| f.contains("BytecodeVerificationRemote")) && one.iter().any(|f| f.contains("G1")));
    }

    #[test]
    fn startup_follows_the_processors_in_automatic_mode() {
        let quick = vec!["-XX:TieredStopAtLevel=1".to_string()];
        for processors in 1..=QUICK_COMPILER_MAX_PROCESSORS {
            assert_eq!(startup_flags(StartupMode::Auto, processors, 16000).flags, quick, "{processors} cores");
        }
        let above = startup_flags(StartupMode::Auto, QUICK_COMPILER_MAX_PROCESSORS + 1, 16000);
        assert!(above.flags.is_empty());
        assert!(above.note.contains("full performance"));
        // a computer a little larger that is short of memory takes the quick compiler too
        assert_eq!(startup_flags(StartupMode::Auto, SHORT_OF_MEMORY_MAX_PROCESSORS, 2000).flags, quick);
        assert!(startup_flags(StartupMode::Auto, SHORT_OF_MEMORY_MAX_PROCESSORS + 1, 2000).flags.is_empty());
        // unknown memory is not "short"
        assert!(startup_flags(StartupMode::Auto, 8, 0).flags.is_empty());
        assert!(startup_flags(StartupMode::Auto, 16, 16000).flags.is_empty());
        assert!(startup_flags(StartupMode::Auto, 2, 16000).note.contains("quick compiler (2 cores)"));
    }

    #[test]
    fn the_setting_forces_or_forbids_the_quick_compiler() {
        assert_eq!(startup_flags(StartupMode::Fast, 32, 64000).flags, vec!["-XX:TieredStopAtLevel=1".to_string()]);
        assert!(startup_flags(StartupMode::Full, 1, 500).flags.is_empty());
        assert_eq!(StartupMode::parse("FAST"), StartupMode::Fast);
        assert_eq!(StartupMode::parse(" full "), StartupMode::Full);
        assert_eq!(StartupMode::parse(""), StartupMode::Auto);
        assert_eq!(StartupMode::parse("something else"), StartupMode::Auto);
    }

    #[test]
    fn the_players_own_compiler_arguments_win() {
        // an argument that says the opposite of the quick compiler is left as it is
        let mut args = vec!["-XX:TieredStopAtLevel=4".to_string()];
        add_missing(&mut args, &startup_flags(StartupMode::Fast, 2, 16000).flags);
        assert_eq!(args, vec!["-XX:TieredStopAtLevel=4".to_string()]);
    }

    #[test]
    fn g1_on_every_computer_and_the_games_own_collections_alongside() {
        // one processor too: Java's own choice there (Serial) froze the game while worlds loaded
        assert_eq!(performance_flags(4096, 1), performance_flags(4096, 8));
        let flags = performance_flags(4096, 8);
        assert!(flags.contains(&"-XX:+UseG1GC".to_string()));
        assert!(flags.contains(&"-XX:G1HeapRegionSize=32M".to_string()));
        // 1.8.9 and 1.12.2 call System.gc() at every world change: concurrent instead of a full stop
        assert!(flags.contains(&"-XX:+ExplicitGCInvokesConcurrent".to_string()));
        // the experimental unlock comes before the experimental option
        let unlock = flags.iter().position(|f| f == "-XX:+UnlockExperimentalVMOptions").unwrap();
        let new_size = flags.iter().position(|f| f.starts_with("-XX:G1NewSizePercent")).unwrap();
        assert!(unlock < new_size);
        // nothing that costs memory on a small computer
        assert!(!flags.iter().any(|f| f.starts_with("-Xms") || f.contains("AlwaysPreTouch") || f.contains("AOTCache")));
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

#[cfg(test)]
mod memory_arg_tests {
    use super::*;

    #[test]
    fn memory_sizes_read_like_java_reads_them() {
        assert_eq!(memory_mb("4G"), Some(4096));
        assert_eq!(memory_mb("4g"), Some(4096));
        assert_eq!(memory_mb("512m"), Some(512));
        assert_eq!(memory_mb("1048576k"), Some(1024));
        assert_eq!(memory_mb("2147483648"), Some(2048));
        assert_eq!(memory_mb("1t"), Some(1024 * 1024));
        for bad in ["", "G", "4x", "four", "-1G", "99999999999999999999999G"] {
            assert_eq!(memory_mb(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_starting_heap_bigger_than_the_game_may_use_is_left_out() {
        let mut args = split("-Xms4G -XX:+UseG1GC -Dx=1");
        let note = drop_oversized_initial_heap(&mut args, 2560);
        assert!(note.unwrap().contains("-Xms4G"));
        assert_eq!(args, vec!["-XX:+UseG1GC", "-Dx=1"]);
        // one that fits stays
        let mut args = split("-Xms1G -XX:+UseG1GC");
        assert!(drop_oversized_initial_heap(&mut args, 2560).is_none());
        assert_eq!(args.len(), 2);
        // a -Xmx of their own decides, as it does in Java (the last one wins)
        let mut args = split("-Xmx8G -Xms6G");
        assert!(drop_oversized_initial_heap(&mut args, 2560).is_none());
        let mut args = split("-Xmx8G -Xmx1G -Xms6G");
        assert!(drop_oversized_initial_heap(&mut args, 2560).is_some());
        // values Java can't read are left for Java to complain about
        let mut args = split("-Xmsbanana");
        assert!(drop_oversized_initial_heap(&mut args, 2560).is_none());
    }
}
