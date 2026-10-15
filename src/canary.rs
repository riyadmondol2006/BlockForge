//! Checks that the services BlockForge installs mod loaders from still answer the way the launcher expects. Run it
//! on a schedule (for example daily in CI); a failed check means an install could break. This is the Rust form of
//! `scripts/loader-canary.sh`, with the same checks in the same order.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

const USER_AGENT: &str = "BlockForge-canary";
const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// One thing that was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CanaryReport {
    /// The Minecraft version the checks used.
    pub minecraft_version: String,
    pub checks: Vec<Check>,
}

impl CanaryReport {
    pub fn healthy(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.ok)
    }

    /// The lines the script prints: `ok    name` or `FAIL  name`, then a verdict.
    pub fn render(&self) -> String {
        let mut out = format!(
            "Minecraft version: {}\n",
            if self.minecraft_version.is_empty() { "unknown" } else { &self.minecraft_version }
        );
        for c in &self.checks {
            out.push_str(&format!("{}{}\n", if c.ok { "ok    " } else { "FAIL  " }, c.name));
        }
        out.push_str(if self.healthy() {
            "All loader services look healthy.\n"
        } else {
            "At least one loader service doesn't look right, installs may fail.\n"
        });
        out
    }
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .context("building the HTTP client")
}

async fn fetch(c: &reqwest::Client, url: &str) -> Result<String> {
    let r = c.get(url).send().await?.error_for_status()?;
    Ok(r.text().await?)
}

async fn fetch_json(c: &reqwest::Client, url: &str) -> Result<Value> {
    Ok(serde_json::from_str(&fetch(c, url).await?)?)
}

async fn head_ok(c: &reqwest::Client, url: &str) -> bool {
    c.head(url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
}

/// Runs every check. `minecraft_version` is the version to check, or `None` for the newest release.
pub async fn run(minecraft_version: Option<&str>) -> Result<CanaryReport> {
    let c = client()?;
    let manifest = fetch_json(&c, MANIFEST).await.ok();
    let mc = match minecraft_version {
        Some(v) => v.to_string(),
        None => manifest
            .as_ref()
            .and_then(|m| m.pointer("/latest/release"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default(),
    };
    let mut report = CanaryReport { minecraft_version: mc.clone(), checks: Vec::new() };
    if mc.is_empty() {
        report.checks.push(Check { name: "could not read the Minecraft version manifest".into(), ok: false });
        return Ok(report);
    }
    let mut push = |name: String, ok: bool| report.checks.push(Check { name, ok });

    push(
        format!("Mojang version manifest lists {mc}"),
        manifest
            .as_ref()
            .and_then(|m| m.get("versions"))
            .and_then(Value::as_array)
            .is_some_and(|v| v.iter().any(|x| x.get("id").and_then(Value::as_str) == Some(mc.as_str()))),
    );

    let fabric = fetch_json(&c, &format!("https://meta.fabricmc.net/v2/versions/loader/{mc}")).await.ok();
    let first_fabric = fabric
        .as_ref()
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|e| e.pointer("/loader/version"))
        .and_then(Value::as_str)
        .map(str::to_string);
    push(
        format!("Fabric has loader versions for {mc}"),
        fabric.as_ref().and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty()) && first_fabric.is_some(),
    );

    let profile_ok = match &first_fabric {
        Some(v) => fetch(&c, &format!("https://meta.fabricmc.net/v2/versions/loader/{mc}/{v}/profile/json"))
            .await
            .is_ok_and(|t| t.contains("mainClass")),
        None => false,
    };
    push(format!("Fabric profile for {mc} has libraries and a main class"), profile_ok);

    let quilt = fetch_json(&c, &format!("https://meta.quiltmc.org/v3/versions/loader/{mc}")).await.ok();
    push(
        format!("Quilt has loader versions for {mc}"),
        quilt.and_then(|v| v.as_array().map(|a| !a.is_empty())).unwrap_or(false),
    );

    push(
        "Forge promotions are published".into(),
        fetch(&c, "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")
            .await
            .is_ok_and(|t| t.contains("\"promos\"")),
    );
    push(
        "NeoForge maven metadata is published".into(),
        fetch(&c, "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml")
            .await
            .is_ok_and(|t| t.contains("<version>")),
    );
    push(
        "Fabric maven serves its loader jar".into(),
        head_ok(&c, "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.15.7/fabric-loader-0.15.7.jar").await,
    );
    push(
        "Minecraft libraries host answers".into(),
        head_ok(&c, "https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar").await,
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_reads_like_the_script() {
        let r = CanaryReport {
            minecraft_version: "1.21".into(),
            checks: vec![Check { name: "A".into(), ok: true }, Check { name: "B".into(), ok: false }],
        };
        assert!(!r.healthy());
        let text = r.render();
        assert!(text.contains("Minecraft version: 1.21") && text.contains("ok    A") && text.contains("FAIL  B"));
        assert!(text.ends_with("installs may fail.\n"));
        let good =
            CanaryReport { minecraft_version: String::new(), checks: vec![Check { name: "A".into(), ok: true }] };
        assert!(good.healthy() && good.render().contains("unknown") && good.render().ends_with("healthy.\n"));
        assert!(!CanaryReport::default().healthy());
    }
}
