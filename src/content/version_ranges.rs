//! Checks a Minecraft version against the version requirements mods write in their metadata: Fabric and Quilt
//! style (`>=1.21 <1.22`, `~1.20.1`, `1.21.x`) and Forge's Maven ranges (`[1.20.1,1.21)`).
//! It is only used to warn, so anything it can't read counts as unknown (`None`) rather than wrong.

/// Whether a Minecraft version meets a requirement: `Some(true)` or `Some(false)`, or `None` when the requirement
/// or the version can't be read.
pub fn matches(requirement: Option<&str>, version: Option<&str>) -> Option<bool> {
    let (requirement, version) = (requirement?, version?);
    let req = requirement.trim();
    if req.is_empty() || req.contains("${") {
        return None;
    }
    parse(version)?;
    if req.starts_with('[') || req.starts_with('(') {
        return maven(req, version);
    }
    let mut any = None;
    for alternative in req.split("||") {
        let one = all(alternative.trim(), version);
        if one == Some(true) {
            return Some(true);
        }
        if one.is_some() {
            any = Some(false);
        }
    }
    any
}

/// Several requirements, any of which is enough (Fabric's array form).
pub fn matches_any<S: AsRef<str>>(requirements: &[S], version: &str) -> Option<bool> {
    let mut result = None;
    for requirement in requirements {
        let one = matches(Some(requirement.as_ref()), Some(version));
        if one == Some(true) {
            return Some(true);
        }
        if one.is_some() {
            result = Some(false);
        }
    }
    result
}

/// Space separated predicates that must all hold.
fn all(predicates: &str, version: &str) -> Option<bool> {
    if predicates.is_empty() {
        return None;
    }
    for predicate in predicates.split_whitespace() {
        match self::predicate(predicate, version) {
            None => return None,
            Some(false) => return Some(false),
            Some(true) => {}
        }
    }
    Some(true)
}

fn predicate(p: &str, version: &str) -> Option<bool> {
    if p == "*" || p == "x" {
        return Some(true);
    }
    let v = parse(version)?;
    if let Some(rest) = p.strip_prefix(">=") {
        return parse(rest).map(|b| compare(&v, &b) >= 0);
    }
    if let Some(rest) = p.strip_prefix("<=") {
        return parse(rest).map(|b| compare(&v, &b) <= 0);
    }
    if let Some(rest) = p.strip_prefix('>') {
        return parse(rest).map(|b| compare(&v, &b) > 0);
    }
    if let Some(rest) = p.strip_prefix('<') {
        return parse(rest).map(|b| compare(&v, &b) < 0);
    }
    if let Some(rest) = p.strip_prefix('~') {
        // same major and minor, at least this patch
        let b = parse(rest)?;
        return Some(compare(&v, &b) >= 0 && part(&v, 0) == part(&b, 0) && part(&v, 1) == part(&b, 1));
    }
    if let Some(rest) = p.strip_prefix('^') {
        let b = parse(rest)?;
        // for Minecraft's 1.x versions the minor number is what really changes, so ^1.20 stays on 1.20
        let same_line = if part(&b, 0) == 1 { part(&v, 1) == part(&b, 1) } else { part(&v, 0) == part(&b, 0) };
        return Some(compare(&v, &b) >= 0 && part(&v, 0) == part(&b, 0) && same_line);
    }
    let body = p.strip_prefix('=').unwrap_or(p);
    if body.ends_with(".x") || body.ends_with(".*") || body.ends_with(".X") {
        let prefix = parse(&body[..body.len() - 2])?;
        return Some(prefix.iter().enumerate().all(|(i, n)| part(&v, i) == *n));
    }
    parse(body).map(|b| compare(&v, &b) == 0)
}

/// Maven ranges: `[a,b)` and friends, several joined by commas meaning any of them.
fn maven(req: &str, version: &str) -> Option<bool> {
    let v = parse(version)?;
    let chars: Vec<char> = req.chars().collect();
    let mut ranges: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '[' | '(' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    ranges.push(chars[start..=i].iter().collect());
                }
            }
            _ => {}
        }
    }
    if ranges.is_empty() {
        return None;
    }
    for range in &ranges {
        let low_inclusive = range.starts_with('[');
        let high_inclusive = range.ends_with(']');
        let inner = &range[1..range.len() - 1];
        let bounds: Vec<&str> = inner.split(',').collect();
        if bounds.len() == 1 {
            let exact = parse(bounds[0].trim())?;
            if compare(&v, &exact) == 0 {
                return Some(true);
            }
            continue;
        }
        let low = bounds[0].trim();
        let high = bounds[1].trim();
        let mut ok = true;
        if !low.is_empty() {
            let l = parse(low)?;
            let c = compare(&v, &l);
            ok = if low_inclusive { c >= 0 } else { c > 0 };
        }
        if ok && !high.is_empty() {
            let h = parse(high)?;
            let c = compare(&v, &h);
            ok = if high_inclusive { c <= 0 } else { c < 0 };
        }
        if ok {
            return Some(true);
        }
    }
    Some(false)
}

/// "1.20.1" as `[1, 20, 1]`; text after a dash (pre-releases) or a plus is ignored. `None` if it isn't a numbered
/// version.
pub fn parse(version: &str) -> Option<Vec<u32>> {
    let mut v = version.trim();
    if let Some(dash) = v.find('-').filter(|d| *d > 0) {
        v = &v[..dash];
    }
    if let Some(plus) = v.find('+').filter(|p| *p > 0) {
        v = &v[..plus];
    }
    if v.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = v.split('.').collect();
    // like Java's String.split, trailing empty pieces are dropped ("1.20." is 1.20)
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    let mut out = Vec::with_capacity(parts.len());
    for p in parts {
        if p.is_empty() || p.len() > 6 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out.push(p.parse().ok()?);
    }
    Some(out)
}

fn part(v: &[u32], i: usize) -> u32 {
    v.get(i).copied().unwrap_or(0)
}

/// Orders two parsed versions; missing parts count as zero, so 1.20 equals 1.20.0.
pub fn compare(a: &[u32], b: &[u32]) -> i32 {
    for i in 0..a.len().max(b.len()) {
        match part(a, i).cmp(&part(b, i)) {
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Equal => {}
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(req: &str, version: &str) -> Option<bool> {
        matches(Some(req), Some(version))
    }

    #[test]
    fn fabric_style_predicates() {
        assert_eq!(m(">=1.21 <1.22", "1.21.4"), Some(true));
        assert_eq!(m(">=1.21 <1.22", "1.22"), Some(false));
        assert_eq!(m("~1.20.1", "1.20.4"), Some(true));
        assert_eq!(m("~1.20.1", "1.21"), Some(false));
        assert_eq!(m("1.21.x", "1.21.11"), Some(true));
        assert_eq!(m("1.21.x", "26.3"), Some(false));
        assert_eq!(m("*", "26.3"), Some(true));
        assert_eq!(m("1.20", "1.20.0"), Some(true));
        assert_eq!(m("^1.20.1", "1.20.6"), Some(true));
        assert_eq!(m("^1.20.1", "1.21"), Some(false));
        assert_eq!(m(">=26.1", "26.3"), Some(true));
        assert_eq!(m("1.19.2 || >=1.20", "1.20.1"), Some(true));
        assert_eq!(matches_any(&["1.20.1", "1.20.4"], "1.20.4"), Some(true));
        assert_eq!(matches_any(&["1.20.1", "1.20.4"], "1.21"), Some(false));
    }

    #[test]
    fn maven_ranges() {
        assert_eq!(m("[1.20.1,1.21)", "1.20.4"), Some(true));
        assert_eq!(m("[1.20.1,1.21)", "1.21"), Some(false));
        assert_eq!(m("[1.20.1]", "1.20.1"), Some(true));
        assert_eq!(m("[1.20.1]", "1.20.2"), Some(false));
        assert_eq!(m("[1.21,)", "26.3"), Some(true));
        assert_eq!(m("(1.20,1.20.4]", "1.20"), Some(false));
        assert_eq!(m("[1.16,1.17),[1.18,1.19)", "1.18.2"), Some(true));
    }

    #[test]
    fn unreadable_means_unknown() {
        assert_eq!(m("${minecraft_version}", "1.20.1"), None);
        assert_eq!(m(">=1.20", "25w14a"), None);
        assert_eq!(matches(None, Some("1.20.1")), None);
        assert_eq!(m("", "1.20.1"), None);
        assert_eq!(m(">=1.21-alpha.24.10.a", "1.21.4"), Some(true));
    }

    #[test]
    fn parsing_edge_cases_match_the_java_edition() {
        assert_eq!(parse("1.20.").unwrap(), vec![1, 20]);
        assert_eq!(parse("1..2"), None);
        assert_eq!(parse("-1.2"), None);
        assert_eq!(parse("1.2+build.5").unwrap(), vec![1, 2]);
        assert_eq!(parse("1234567.1"), None);
        assert_eq!(compare(&[1, 20], &[1, 20, 0]), 0);
        assert_eq!(compare(&[1, 21], &[1, 20, 9]), 1);
    }
}
