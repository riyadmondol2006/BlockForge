use super::*;
use crate::testutil::{Reply, Server, scratch};
use msixvc::{streaming::HttpRead, xvd::SegmentFile};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use xodus::tokens::store::TokenBackend;

#[test]
fn scoped_credentials_survive_restart_and_user_logout() {
    let root = scratch("store-slot");
    {
        let slot = Slot::open(root.clone()).unwrap();
        slot.backend.set("dev_license", b"device-fixture").unwrap();
        slot.backend.set("device-tokens", b"sts-fixture").unwrap();
        slot.backend.set("user-DA", b"user-fixture").unwrap();
        assert!(matches!(Slot::open(root.clone()).err().unwrap().kind, ErrorKind::Busy));
        slot.sign_out(SignOutMode::User).unwrap();
        assert_eq!(slot.backend.get("dev_license").unwrap().unwrap(), b"device-fixture");
        assert!(slot.backend.get("user-DA").unwrap().is_none());
    }
    let slot = Slot::open(root.clone()).unwrap();
    assert!(slot.backend.get("device-tokens").unwrap().is_some());
    let other = Slot::open(root.join("other")).unwrap();
    assert!(other.backend.get("dev_license").unwrap().is_none());
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn corrupt_or_lost_credentials_never_reprovision() {
    let root = scratch("corrupt-store");
    drop(Slot::open(root.clone()).unwrap());
    std::fs::write(root.join("credentials.json"), b"broken").unwrap();
    assert!(matches!(Slot::open(root.clone()).err().unwrap().kind, ErrorKind::SignInRequired));
    std::fs::remove_file(root.join("credentials.json")).unwrap();
    assert!(Slot::open(root.clone()).is_err());
    tokens::atomic(&root.join("credentials.json"), b"{}").unwrap();
    let slot = Slot::open(root.clone()).unwrap();
    tokens::atomic(&root.join("provisioning"), b"1").unwrap();
    let error = device::ensure(&reqwest::Client::new(), &slot, &Cancel::default()).await.unwrap_err();
    assert!(matches!(error.kind, ErrorKind::SignInRequired));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn a_saved_registration_is_never_reported_as_interrupted() {
    // A slot whose marker outlived a registration that was saved (as every successful sign-in left it before the
    // marker was cleared on success): signed in, not "registration interrupted".
    let base = scratch("store-marker");
    let id = StoreId(uuid::Uuid::new_v4());
    let root = base.join("stores").join(id.0.to_string());
    std::fs::create_dir_all(&root).unwrap();
    let slot = Slot::open(root.clone()).unwrap();
    tokens::atomic(&root.join("provisioning"), b"1").unwrap();
    assert!(matches!(super::cached_state(&base, &id), SignInState::NeedsSignIn { .. }), "nothing saved yet");
    slot.backend.set("dev_license", b"device-fixture").unwrap();
    assert!(!matches!(super::cached_state(&base, &id), SignInState::NeedsSignIn { .. }));
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
fn rejects_credential_symlinks_and_unsafe_package_paths() {
    let root = scratch("store-symlink");
    drop(Slot::open(root.clone()).unwrap());
    std::fs::remove_file(root.join("credentials.json")).unwrap();
    std::os::unix::fs::symlink(root.join("initialized"), root.join("credentials.json")).unwrap();
    assert!(Slot::open(root.clone()).is_err());
    for name in [
        "../x",
        "a/../../x",
        "a\\..\\x",
        "/absolute",
        "C:\\game.exe",
        "a//x",
        "NUL",
        "com1.txt",
        "a/LPT9",
        "a/x.",
        "x:stream",
        ".xodus-streaming.msixvc",
        "a\0b",
    ] {
        assert!(download::safe_name(name).is_err(), "{name:?}");
    }
    assert_eq!(download::safe_name("Content\\data.bin").unwrap(), "Content/data.bin");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn helper_protocol_is_bounded_versioned_and_cancel_is_not_success() {
    let bytes = ipc::encode(ipc::Message::Cancelled).unwrap();
    assert!(matches!(ipc::decode(&bytes[4..]).unwrap(), ipc::Message::Cancelled));
    assert!(matches!(ipc::decode(br#"{"version":2,"message":{"type":"Complete"}}"#).unwrap(), ipc::Message::Complete));
    // A helper of another version (one without the Xbox step, say) is refused, not half understood.
    assert!(ipc::decode(br#"{"version":1,"message":{"type":"Complete"}}"#).is_err());
    assert!(ipc::decode(br#"{"version":3,"message":{"type":"Complete"}}"#).is_err());
    assert!(ipc::decode(br#"{"version":2,"message":{"type":"Unknown"}}"#).is_err());
    assert!(ipc::decode(&vec![b' '; ipc::LIMIT + 1]).is_err());
    for url in [
        "http://login.live.com/",
        "https://login.live.com.evil/",
        "https://evil@login.live.com/",
        "https://login.live.com:444/",
        "file:///etc/passwd",
    ] {
        assert!(!ipc::allowed_url(url));
    }
    assert!(!ipc::credential_origin("https://login.live.com/ppsecure/post.srf.evil"));
    let first = ipc::headers("correlation");
    let second = ipc::headers("correlation");
    assert_eq!(first, second);
    assert_eq!(first.len(), 9);
    assert!(ipc::host_context("\";alert(1)//").contains("\\\""));
}
/// A frame of the sign-in page can post to the helper's IPC handler as if it were the page: only a message that
/// carries the nonce of the helper's own ServerData read becomes `Credentials`, and only once.
#[test]
fn page_credentials_need_the_helpers_nonce() {
    const PAGE: &str = "https://login.live.com/ppsecure/post.srf";
    const NONCE: &str = "0123456789abcdef0123456789abcdef";
    let fields = |nonce: Option<&str>| {
        let mut m = serde_json::Map::new();
        for f in ipc::DA_FIELDS {
            m.insert(f.to_string(), format!("value-of-{f}").into());
        }
        if let Some(n) = nonce {
            m.insert("nonce".into(), n.into());
        }
        serde_json::Value::Object(m).to_string()
    };
    // The helper's own script is what carries the nonce, and it only accepts one that is plain letters and digits.
    let script = ipc::read_server_data(NONCE).unwrap();
    assert!(script.contains(&format!("\"nonce\":\"{NONCE}\"")) && script.contains("ServerData[\"sDAToken\"]"));
    assert!(ipc::read_server_data("x\"+alert(1)+\"0123456789").is_none() && ipc::read_server_data("short").is_none());
    // With the nonce: exactly the DA fields, nothing else the page added.
    let mut extra: serde_json::Value = serde_json::from_str(&fields(Some(NONCE))).unwrap();
    extra["password"] = "hunter2".into();
    for body in [fields(Some(NONCE)), extra.to_string()] {
        let Some(ipc::Message::Credentials { data }) = ipc::credentials_from_page(&body, PAGE, Some(NONCE)) else {
            panic!("credentials with the nonce are accepted");
        };
        let keys: std::collections::BTreeSet<_> = data.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ipc::DA_FIELDS.iter().map(|f| f.to_string()).collect());
    }
    // Without a nonce, with a wrong one, with none expected, or from the wrong place: ignored.
    assert!(ipc::credentials_from_page(&fields(None), PAGE, Some(NONCE)).is_none());
    assert!(ipc::credentials_from_page(&fields(Some("fedcba9876543210fedcba9876543210")), PAGE, Some(NONCE)).is_none());
    assert!(ipc::credentials_from_page(&fields(Some("")), PAGE, Some(NONCE)).is_none());
    assert!(ipc::credentials_from_page(&fields(Some(NONCE)), PAGE, None).is_none());
    assert!(ipc::credentials_from_page(&fields(Some(NONCE)), PAGE, Some("short")).is_none());
    assert!(ipc::credentials_from_page(&fields(Some(NONCE)), "https://evil.example/", Some(NONCE)).is_none());
    assert!(ipc::credentials_from_page(&fields(Some(NONCE)), "https://login.live.com.evil/", Some(NONCE)).is_none());
    // A non-string nonce, a missing DA field, junk and an oversized body are all refused.
    let mut number: serde_json::Value = serde_json::from_str(&fields(Some(NONCE))).unwrap();
    number["nonce"] = 5.into();
    assert!(ipc::credentials_from_page(&number.to_string(), PAGE, Some(NONCE)).is_none());
    let mut missing: serde_json::Value = serde_json::from_str(&fields(Some(NONCE))).unwrap();
    missing.as_object_mut().unwrap().remove("K");
    assert!(ipc::credentials_from_page(&missing.to_string(), PAGE, Some(NONCE)).is_none());
    assert!(ipc::credentials_from_page("not json", PAGE, Some(NONCE)).is_none());
    assert!(ipc::credentials_from_page(&" ".repeat(ipc::LIMIT + 1), PAGE, Some(NONCE)).is_none());
    // A page cannot make the helper say it has an Xbox code, whatever it posts.
    let xbox = r#"{"type":"XboxCode","code":"c","state":"s"}"#;
    assert!(ipc::credentials_from_page(xbox, PAGE, Some(NONCE)).is_none());
}
#[test]
fn missing_token_key_requests_signin() {
    let manager = xodus::tokens::TokenManager::with_memory();
    let error = license::legacy(manager.get_device_sts_token()).err().unwrap();
    assert!(matches!(error.kind, ErrorKind::SignInRequired));
}
#[tokio::test]
async fn licensing_http_status_and_reason_and_malformed_responses() {
    use xodus::licensing::content::{LicenseContentError, get_license_content_at};
    let server = Server::start().await;
    for status in [401, 403] {
        server.route(
            "/license",
            Reply {
                status,
                ..Reply::ok(
                    br#"{"code":"Denied","innererror":{"code":"InvalidMsaTicketFormat","message":"bad format"}}"#
                        .to_vec(),
                )
            },
        );
        let error = get_license_content_at(
            &reqwest::Client::new(),
            &format!("{}/license", server.base),
            "fixture-device".into(),
            "fixture-user".into(),
            "fixture-ref".into(),
            "fixture-content".into(),
            "neutral".into(),
        )
        .await
        .err()
        .unwrap();
        assert!(
            matches!(&error, LicenseContentError::Refused { status: s, reason } if *s == status && reason.contains("InvalidMsaTicketFormat"))
        );
        let error = license::license_error(error);
        assert!(matches!(error.kind, ErrorKind::LicenseUnavailable));
        assert!(!error.message.contains("fixture"));
    }
    for body in [
        r#"{"license":{"keys":[],"leases":[]}}"#,
        r#"{"license":{"keys":[{"value":"!!!"}],"leases":[]}}"#,
        r#"{"license":{"keys":[{"value":"PE5vcGUvPg=="}],"leases":[]}}"#,
    ] {
        server.route("/license", Reply::ok(body));
        assert!(
            get_license_content_at(
                &reqwest::Client::new(),
                &format!("{}/license", server.base),
                "fixture-device".into(),
                "fixture-user".into(),
                "fixture-ref".into(),
                "fixture-content".into(),
                "neutral".into()
            )
            .await
            .is_err()
        );
    }
}
#[tokio::test]
async fn truncated_and_mutated_files_are_not_incremental_hits() {
    let root = scratch("incremental");
    let path = root.join("file");
    tokio::fs::write(&path, b"fixture").await.unwrap();
    let segment = SegmentFile { offset: 0, length: 7, data_hashs: vec![[1; 20]], keep_encrypted: false };
    let entry = download::Entry {
        offset: 0,
        length: 7,
        hashes: vec![[1; 20]],
        encrypted: false,
        disk_sha256: download::disk_hash(&path).await.unwrap(),
        local_length: 7,
        patch_revision: None,
    };
    assert!(download::reusable(&path, &entry, &segment).await);
    tokio::fs::write(&path, b"short").await.unwrap();
    assert!(!download::reusable(&path, &entry, &segment).await);
    tokio::fs::write(&path, b"changed").await.unwrap();
    assert!(!download::reusable(&path, &entry, &segment).await);
    assert_eq!(download::disk_length(4097, true), 8192);
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn http_seek_rejects_ignored_range() {
    let server = Server::start().await;
    server.route("/file", Reply::ok(vec![1; 8192]));
    let client = reqwest::Client::new();
    let mut reader = HttpRead::open(client.clone(), format!("{}/file", server.base), Some(|_, _| {})).await.unwrap();
    reader.seek(std::io::SeekFrom::Start(4096)).await.unwrap();
    assert!(reader.read_exact(&mut [0; 16]).await.is_err());
}
#[tokio::test]
async fn cancellation_never_stamps_complete_and_metadata_is_required() {
    let root = scratch("cancel-install");
    let ctx = Context { root: root.clone(), client: reqwest::Client::new(), auth_helper: PathBuf::from("unused") };
    let slot = ctx.slot(&StoreId(uuid::Uuid::new_v4())).await.unwrap();
    let build =
        BuildId { edition: Edition::Release, version: "1.2.3.4".into(), package_id: uuid::Uuid::new_v4().to_string() };
    let cancel = Cancel::default();
    cancel.cancel();
    let error = download::install(
        &ctx,
        &slot,
        catalog::Package { build: build.clone(), url: "http://127.0.0.1:1/unused".into(), bytes: 8192 },
        Arc::new(Progress::default()),
        &cancel,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error.kind, ErrorKind::Cancelled));
    assert!(download::load(&ctx, &build).await.is_err());
    std::fs::remove_dir_all(root).unwrap();
}

/// Interactive only: current owned Release, stable device, incremental re-run, tmpfs MZ/PE header.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "Requires the user's interactive Microsoft Store sign-in and several GB of scratch space"]
async fn real_store_release_scratch() {
    use std::io::{Read, Seek};
    let home = std::env::var_os("HOME").unwrap();
    let root = PathBuf::from(home).join(".cache/bf-bedrock-pkga/real-store");
    let helper = std::env::var_os("BF_BEDROCK_AUTH_HELPER")
        .map(PathBuf::from)
        .expect("Set BF_BEDROCK_AUTH_HELPER to the built helper binary");
    let ctx = Context { root, client: crate::http::client().clone(), auth_helper: helper };
    let id = StoreId(uuid::Uuid::from_u128(0x12f19a463c04464fae822c69d90a6f13));
    let cancel = Cancel::default();
    sign_in(&ctx, id.clone(), Arc::new(Progress::default()), cancel.clone()).await.unwrap();
    let device_fingerprint = {
        let slot = ctx.slot(&id).await.unwrap();
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(slot.backend.get("dev_license").unwrap().unwrap()))
    };
    let installed = install(
        &ctx,
        id.clone(),
        BuildSelection::Latest(Edition::Release),
        Arc::new(Progress::default()),
        cancel.clone(),
    )
    .await
    .unwrap();
    let build = installed.id.clone();
    println!("Installed {build:?}: {} files fetched", installed.downloaded_files);
    drop(installed);
    let rerun_started = std::time::Instant::now();
    let installed = install(
        &ctx,
        id.clone(),
        BuildSelection::Latest(Edition::Release),
        Arc::new(Progress::default()),
        cancel.clone(),
    )
    .await
    .unwrap();
    assert_eq!(installed.id, build);
    println!("Second run took {:?}", rerun_started.elapsed());
    println!(
        "Unchanged Release re-run: {} files fetched; inspect zero-hash metadata entries if nonzero",
        installed.downloaded_files
    );
    let verifiable = installed.receipt.files.values().all(|e| !e.hashes.is_empty());
    if verifiable {
        assert_eq!(installed.downloaded_files, 0);
    }
    let staged = stage(&ctx, id.clone(), &installed, cancel).await.unwrap();
    // Read 64 + 4 header bytes of each image, nothing more; nothing is copied off tmpfs.
    let header = |image: &super::stage::StagedImage| -> bool {
        let mut file = image.file.try_clone().unwrap();
        file.rewind().unwrap();
        let mut dos = [0; 64];
        if file.read_exact(&mut dos).is_err() || &dos[..2] != b"MZ" {
            return false;
        }
        let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
        let mut signature = [0; 4];
        file.seek(std::io::SeekFrom::Start(offset)).is_ok()
            && file.read_exact(&mut signature).is_ok()
            && &signature == b"PE\0\0"
    };
    let mut executables = 0;
    for image in &staged.images {
        let pe = header(image);
        println!(
            "staged {} ({} bytes): MZ/PE header {}",
            image.relative,
            image.file.metadata().unwrap().len(),
            if pe { "ok" } else { "absent" }
        );
        if image.relative.to_ascii_lowercase().ends_with(".exe") {
            assert!(pe, "{} is not a PE image after decryption", image.relative);
            executables += 1;
        }
    }
    assert!(executables > 0, "no encrypted executable was staged");
    drop(staged);
    // A second helper process must use the same provisioned device, never register a fresh one.
    sign_in(&ctx, id.clone(), Arc::new(Progress::default()), Cancel::default()).await.unwrap();
    let slot = ctx.slot(&id).await.unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(device_fingerprint, hex::encode(Sha256::digest(slot.backend.get("dev_license").unwrap().unwrap())));
    println!(
        "Release installed and staged; only MZ/PE headers inspected; Store device persisted. Scratch root retained for incremental/restart checks."
    );
}

#[tokio::test]
async fn latest_catalog_uses_edition_identity_and_rejects_untrusted_cdn() {
    let server = Server::start().await;
    let package_id = uuid::Uuid::new_v4().to_string();
    let content = catalog::content_id(Edition::Release);
    let path = format!("/GetBasePackage/{content}");
    let mut details = serde_json::json!({
        "PackageFound":true, "ContentId":content, "VersionId":package_id,
        "Version":"1.2.3.4", "UpdatePredownload":false, "AvailabilityDate":"fixture",
        "PackageFiles":[{"ContentId":content,"VersionId":package_id,"FileName":"fixture.msixvc", "FileSize":8192,"FileHash":"fixture", "KeyBlob":"", "CdnRootPaths":["http://assets1.xboxlive.com/"], "BackgroundCdnRootPaths":[],"RelativeUrl":format!("{content}/fixture.msixvc"),"UpdateType":0,"LicenseUsageType":0,"ModifiedDate":"fixture"}]
    });
    server.route(&path, Reply::ok(details.to_string()));
    let package =
        catalog::resolve(&reqwest::Client::new(), &server.base, "fixture".into(), Edition::Release).await.unwrap();
    assert_eq!(package.build.version, "1.2.3.4");
    assert!(
        package.url.starts_with("http://assets1.xboxlive.com/"),
        "the CDN scheme is kept: its hosts have no matching TLS certificate"
    );
    details["PackageFiles"][0]["CdnRootPaths"] = serde_json::json!(["https://assets1.xboxlive.com.evil/"]);
    server.route(&path, Reply::ok(details.to_string()));
    assert!(catalog::resolve(&reqwest::Client::new(), &server.base, "fixture".into(), Edition::Release).await.is_err());
    assert_ne!(catalog::content_id(Edition::Release), catalog::content_id(Edition::Preview));
}
#[tokio::test]
async fn mid_download_cancel_removes_pending_generation_and_missing_metadata_refuses_load() {
    let root = scratch("mid-download-cancel");
    let server = Server::start().await;
    let url = server.route("/slow", Reply::ok(vec![0; 16384]).slow(1000));
    let ctx = Context { root: root.clone(), client: reqwest::Client::new(), auth_helper: PathBuf::from("unused") };
    let slot = ctx.slot(&StoreId(uuid::Uuid::new_v4())).await.unwrap();
    let build =
        BuildId { edition: Edition::Release, version: "1.2.3.4".into(), package_id: uuid::Uuid::new_v4().to_string() };
    let cancel = Cancel::default();
    let cancel_worker = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        cancel_worker.cancel();
    });
    let error = download::install(
        &ctx,
        &slot,
        catalog::Package { build: build.clone(), url, bytes: 16384 },
        Arc::new(Progress::default()),
        &cancel,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error.kind, ErrorKind::Cancelled));
    let parent = root.join("builds/release/1.2.3.4");
    assert!(std::fs::read_dir(&parent).unwrap().next().is_none());
    let build_root = parent.join(&build.package_id);
    std::fs::create_dir_all(build_root.join("content")).unwrap();
    let receipt = download::Receipt {
        schema: 1,
        adapter: 1,
        build: build.clone(),
        content_id: catalog::content_id(Edition::Release).into(),
        files: Default::default(),
    };
    std::fs::write(build_root.join("install.json"), serde_json::to_vec(&receipt).unwrap()).unwrap();
    let error = download::load(&ctx, &build).await.err().unwrap();
    assert!(matches!(error.kind, ErrorKind::Integrity));
    assert!(error.message.contains("metadata"));
    std::fs::remove_dir_all(root).unwrap();
}

const STATE: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn the_xbox_redirect_gives_only_its_own_code() {
    let at = |rest: &str| format!("{}{rest}", ipc::XBOX_REDIRECT);
    let code = |m: Option<ipc::Message>| match m {
        Some(ipc::Message::XboxCode { code, state }) => Ok((code, state)),
        Some(ipc::Message::XboxUnavailable { error }) => Err(error),
        None => Err("not the redirect".into()),
        Some(_) => Err("another message".into()),
    };
    // the code, decoded, with the state asked for
    let good = at(&format!("?code=M.C5_BAY.2.U.fake-code%21&state={STATE}&lc=1033"));
    assert_eq!(code(ipc::xbox_capture(&good, STATE)), Ok(("M.C5_BAY.2.U.fake-code!".into(), STATE.into())));
    // another state, none, or a state the parent can't have made: no code, whatever the redirect carries
    let other = "fedcba9876543210fedcba9876543210";
    assert_eq!(code(ipc::xbox_capture(&good, other)), Err("state_mismatch".into()));
    assert_eq!(code(ipc::xbox_capture(&at("?code=fake-code"), STATE)), Err("state_mismatch".into()));
    assert_eq!(code(ipc::xbox_capture(&at("?code=c&state=short"), "short")), Err("state_mismatch".into()));
    // Microsoft's refusals come back as their bare code, never the description (which can quote the request)
    let consent = at(&format!("?error=consent_required&error_description=The+user+fake-code&state={STATE}"));
    assert_eq!(code(ipc::xbox_capture(&consent, STATE)), Err("consent_required".into()));
    let odd = at(&format!("?error=%3Cscript%3E&state={STATE}"));
    assert_eq!(code(ipc::xbox_capture(&odd, STATE)), Err("script".into()));
    assert_eq!(code(ipc::xbox_capture(&at(&format!("?state={STATE}")), STATE)), Err("no_code".into()));
    assert_eq!(code(ipc::xbox_capture(&at(&format!("#code=c&state={STATE}")), STATE)), Ok(("c".into(), STATE.into())));
    // only the exact desktop redirect is read: other hosts, schemes, paths, user-info and ports are just pages
    for url in [
        format!("https://login.live.com.evil.example/oauth20_desktop.srf?code=c&state={STATE}"),
        format!("https://evil.example/https://login.live.com/oauth20_desktop.srf?code=c&state={STATE}"),
        format!("http://login.live.com/oauth20_desktop.srf?code=c&state={STATE}"),
        format!("https://user@login.live.com/oauth20_desktop.srf?code=c&state={STATE}"),
        format!("https://login.live.com:443/oauth20_desktop.srf?code=c&state={STATE}"),
        format!("https://login.live.com/oauth20_desktop.srfx?code=c&state={STATE}"),
        format!("https://login.live.com/oauth20_authorize.srf?code=c&state={STATE}"),
        format!("https://account.live.com/oauth20_desktop.srf?code=c&state={STATE}"),
    ] {
        assert!(ipc::xbox_capture(&url, STATE).is_none(), "{url}");
    }
    assert!(ipc::is_xbox_redirect(&at("")) && !ipc::is_xbox_redirect("about:blank"));
}

#[test]
fn the_window_asks_xbox_with_winegdks_app_and_prefills_only_an_address() {
    let url = ipc::xbox_authorize_url(STATE, Some("owner+bf@outlook.com"));
    assert_eq!(
        url,
        format!(
            "https://login.live.com/oauth20_authorize.srf?client_id=0000000048183522\
             &scope=service%3A%3Auser.auth.xboxlive.com%3A%3AMBI_SSL&response_type=code\
             &redirect_uri=https%3A%2F%2Flogin.live.com%2Foauth20_desktop.srf&state={STATE}\
             &login_hint=owner%2Bbf%40outlook.com"
        )
    );
    assert!(ipc::allowed_url(&url) && ipc::allowed_navigation(&url) && !url.contains("prompt="));
    assert!(!ipc::xbox_authorize_url(STATE, Some("Steve")).contains("login_hint"));
    assert!(ipc::valid_state(STATE) && !ipc::valid_state("a&b=0123456789abcdef") && !ipc::valid_state("abc"));
    // the Store page: the bare page, or the page with a prefilled address, and nothing else may be opened first
    assert_eq!(ipc::login_url(None), ipc::LOGIN);
    assert_eq!(ipc::login_url(Some("<b>@x")), ipc::LOGIN);
    let hinted = ipc::login_url(Some("owner@outlook.com"));
    assert_eq!(hinted, format!("{}&username=owner%40outlook.com", ipc::LOGIN));
    assert!(ipc::is_login_url(ipc::LOGIN) && ipc::is_login_url(&hinted));
    for url in [
        format!("{}&username=owner@outlook.com", ipc::LOGIN),
        format!("{}&username=a%40b.c&redirect=https://evil", ipc::LOGIN),
        format!("{}&username=", ipc::LOGIN),
        format!("{}&other=x", ipc::LOGIN),
        "https://login.live.com/".to_string(),
    ] {
        assert!(!ipc::is_login_url(&url), "{url}");
    }
    assert_eq!(ipc::decode_component("a%2"), None);
    assert_eq!(ipc::decode_component("a+b%21").as_deref(), Some("a b!"));
}

#[tokio::test]
async fn the_xbox_step_round_trips_and_every_failure_is_a_fallback() {
    use tokio::io::AsyncWriteExt;
    // The helper's side: answers each XboxAuthorize as told, after a stray Store message.
    async fn helper(mut stream: tokio::net::UnixStream, answers: Vec<&'static str>) -> Vec<Option<String>> {
        let mut hints = Vec::new();
        for answer in answers {
            let Ok(ipc::Message::XboxAuthorize { state, login_hint }) = login::receive(&mut stream).await else {
                panic!("expected an XboxAuthorize");
            };
            assert!(ipc::valid_state(&state));
            hints.push(login_hint);
            let stray = ipc::Message::Credentials { data: serde_json::Value::Null };
            stream.write_all(&ipc::encode(stray).unwrap()).await.unwrap();
            let reply = match answer {
                "code" => {
                    ipc::xbox_capture(&format!("{}?code=fake-code-{state}&state={state}", ipc::XBOX_REDIRECT), &state)
                }
                "consent" => Some(ipc::Message::XboxUnavailable { error: "consent_required".into() }),
                "wrong-state" => Some(ipc::Message::XboxCode { code: "fake-code".into(), state: "x".repeat(32) }),
                "close" => Some(ipc::Message::Cancelled),
                _ => None,
            };
            if let Some(reply) = reply {
                stream.write_all(&ipc::encode(reply).unwrap()).await.unwrap();
            }
        }
        // what the parent sends after the last code
        let end = login::receive(&mut stream).await;
        assert!(matches!(end, Ok(ipc::Message::Complete) | Err(_)));
        hints
    }
    let names = |steps: &[XboxStep]| -> Vec<String> {
        steps
            .iter()
            .map(|s| match s {
                XboxStep::Code(c) => format!("code:{}", c.expose().starts_with("fake-code-")),
                XboxStep::Unavailable(why) => why.clone(),
            })
            .collect()
    };
    let (mut parent, child) = tokio::net::UnixStream::pair().unwrap();
    let h = tokio::spawn(helper(child, vec!["code", "consent", "wrong-state", "code"]));
    let steps = login::xbox_steps(&mut parent, Some("owner@outlook.com"), 4, &Cancel::default()).await;
    parent.write_all(&ipc::encode(ipc::Message::Complete).unwrap()).await.unwrap();
    assert_eq!(names(&steps), ["code:true", "consent_required", "state_mismatch", "code:true"]);
    assert!(h.await.unwrap().iter().all(|h| h.as_deref() == Some("owner@outlook.com")));

    // the window closed: that code and the ones after it are unavailable, and nothing more is asked
    let (mut parent, child) = tokio::net::UnixStream::pair().unwrap();
    let h = tokio::spawn(helper(child, vec!["close"]));
    let steps = login::xbox_steps(&mut parent, None, 3, &Cancel::default()).await;
    drop(parent);
    assert_eq!(names(&steps), ["closed", "closed", "closed"]);
    assert_eq!(h.await.unwrap(), [None]);

    // a cancel while the page waits for the player ends the step at once, with no error
    let (mut parent, child) = tokio::net::UnixStream::pair().unwrap();
    let h = tokio::spawn(async move {
        let mut child = child;
        let _ = login::receive(&mut child).await;
        let _ = login::receive(&mut child).await;
    });
    let cancel = Cancel::default();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        c.cancel();
    });
    let steps = login::xbox_steps(&mut parent, None, 2, &cancel).await;
    assert_eq!(names(&steps), ["cancelled", "cancelled"]);
    drop(parent);
    h.await.unwrap();

    // asked for none (the Store sign-in alone): nothing goes to the window
    let (mut parent, _child) = tokio::net::UnixStream::pair().unwrap();
    assert!(login::xbox_steps(&mut parent, None, 0, &Cancel::default()).await.is_empty());
}

#[tokio::test]
async fn helper_close_without_credentials_is_cancelled() {
    let (mut parent, helper) = tokio::net::UnixStream::pair().unwrap();
    drop(helper);
    let error = login::receive(&mut parent).await.err().unwrap();
    assert!(matches!(error.kind, ErrorKind::Cancelled));
}
#[test]
fn cloned_managers_serialize_token_merges() {
    let root = scratch("token-merges");
    let slot = Slot::open(root.clone()).unwrap();
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let manager = slot.tokens.clone();
            std::thread::spawn(move || {
                manager
                    .save_user_token(format!("fixture-{i}"), xodus::models::secrets::Token::Compact("fixture".into()))
                    .unwrap()
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    for i in 0..16 {
        assert!(slot.tokens.get_user_token_for(&format!("fixture-{i}")).unwrap().is_some());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_device_reset_can_repair_corrupt_credentials() {
    let root = scratch("explicit-device-reset");
    drop(Slot::open(root.clone()).unwrap());
    std::fs::write(root.join("credentials.json"), b"corrupt").unwrap();
    let slot = Slot::open_for_reset(root.clone()).unwrap();
    slot.sign_out(SignOutMode::UserAndDevice).unwrap();
    assert!(slot.backend.get("dev_license").unwrap().is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn recorded_patch_hash_and_length_survive_receipt_reload() {
    let root = scratch("patch-receipt");
    let ctx = Context { root: root.clone(), client: reqwest::Client::new(), auth_helper: PathBuf::from("unused") };
    let id =
        BuildId { edition: Edition::Release, version: "1.2.3.4".into(), package_id: uuid::Uuid::new_v4().to_string() };
    let build_root = root.join("builds/release/1.2.3.4").join(&id.package_id);
    let content = build_root.join("content");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join(download::METADATA), b"synthetic-metadata-placeholder").unwrap();
    std::fs::write(content.join("ui.js"), b"base").unwrap();
    let entry = download::Entry {
        offset: 0,
        length: 4,
        hashes: vec![[1; 20]],
        encrypted: false,
        disk_sha256: download::disk_hash(&content.join("ui.js")).await.unwrap(),
        local_length: 4,
        patch_revision: None,
    };
    let receipt = download::Receipt {
        schema: 1,
        adapter: 1,
        build: id.clone(),
        content_id: catalog::content_id(Edition::Release).into(),
        files: [("ui.js".into(), entry)].into(),
    };
    std::fs::write(build_root.join("install.json"), serde_json::to_vec(&receipt).unwrap()).unwrap();
    let mut build = download::load(&ctx, &id).await.unwrap();
    std::fs::write(content.join("ui.js"), b"patched synthetic UI").unwrap();
    build.record_patch("ui.js", "fixture-v1").await.unwrap();
    drop(build);
    let build = download::load(&ctx, &id).await.unwrap();
    let entry = build.receipt.files.get("ui.js").unwrap();
    assert_eq!(entry.patch_revision.as_deref(), Some("fixture-v1"));
    assert!(
        download::reusable(
            &content.join("ui.js"),
            entry,
            &SegmentFile { offset: 0, length: 4, data_hashs: vec![[1; 20]], keep_encrypted: false }
        )
        .await
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A synthetic PE header: "MZ", the offset of the signature at 0x3c, "PE\0\0" there.
fn synthetic_pe() -> Vec<u8> {
    let mut image = vec![0u8; 0x200];
    image[..2].copy_from_slice(b"MZ");
    image[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    image[0x80..0x84].copy_from_slice(b"PE\0\0");
    image
}

#[test]
fn a_plain_image_is_told_from_ciphertext_by_its_pe_header() {
    assert!(stage::is_plain_pe(&synthetic_pe()));
    let mut moved = synthetic_pe();
    moved[0x3c..0x40].copy_from_slice(&0x100u32.to_le_bytes());
    assert!(!stage::is_plain_pe(&moved), "MZ alone is not enough");
    assert!(!stage::is_plain_pe(&[0x8c; 0x200]), "ciphertext");
    assert!(!stage::is_plain_pe(b"MZ"), "cut off");
    let mut far = synthetic_pe();
    far[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(!stage::is_plain_pe(&far));
}

#[tokio::test]
async fn an_image_shipped_in_the_clear_is_not_decrypted_and_may_be_patched_on_disk() {
    let root = scratch("plain-image");
    let ctx = Context { root: root.clone(), client: reqwest::Client::new(), auth_helper: PathBuf::from("unused") };
    let id =
        BuildId { edition: Edition::Release, version: "1.2.3.4".into(), package_id: uuid::Uuid::new_v4().to_string() };
    let build_root = root.join("builds/release/1.2.3.4").join(&id.package_id);
    let content = build_root.join("content");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join(download::METADATA), b"synthetic-metadata-placeholder").unwrap();
    std::fs::write(content.join("Game.exe"), synthetic_pe()).unwrap();
    std::fs::write(content.join("Other.dll"), [0x8c; 0x200]).unwrap();
    let mut files = std::collections::BTreeMap::new();
    for name in ["Game.exe", "Other.dll"] {
        let entry = download::Entry {
            offset: 0,
            length: 0x200,
            hashes: vec![[1; 20]],
            encrypted: true,
            disk_sha256: download::disk_hash(&content.join(name)).await.unwrap(),
            local_length: 0x200,
            patch_revision: None,
        };
        files.insert(name.to_string(), entry);
    }
    let receipt = download::Receipt {
        schema: 1,
        adapter: 1,
        build: id.clone(),
        content_id: catalog::content_id(Edition::Release).into(),
        files,
    };
    std::fs::write(build_root.join("install.json"), serde_json::to_vec(&receipt).unwrap()).unwrap();
    let mut build = download::load(&ctx, &id).await.unwrap();
    assert_eq!(build.images_to_decrypt().await, vec!["Other.dll".to_string()]);
    assert_eq!(build.decrypted_bytes().await.unwrap(), 0x200);
    let mut patched = synthetic_pe();
    patched[0x100] = 1;
    std::fs::write(content.join("Game.exe"), &patched).unwrap();
    build.record_patch("Game.exe", "stack").await.unwrap();
    assert!(build.record_patch("Other.dll", "stack").await.is_err(), "ciphertext is never patched on disk");
    drop(build);
    let build = download::load(&ctx, &id).await.unwrap();
    assert_eq!(build.patch_revision("Game.exe"), Some("stack"));
    assert!(build.damaged_files(&Cancel::default()).await.unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn staging_memory_budget_requires_valid_available_ram() {
    assert_eq!(stage::memory_available("MemTotal: 100 kB\nMemAvailable: 42 kB\n").unwrap(), 42 * 1024);
    assert!(stage::memory_available("MemTotal: 100 kB\n").is_err());
    assert!(stage::memory_available("MemAvailable: 18446744073709551615 kB").is_err());
}

#[tokio::test]
async fn malformed_range_end_length_and_truncated_body_are_refused() {
    use tokio::io::AsyncWriteExt;
    for (range, length, body) in [("4-7/9", 4, "xxxx"), ("4-8/9", 3, "xxx"), ("4-8/9", 5, "xx")] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {range}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}"
        );
        let worker = tokio::spawn(async move {
            for _ in 0..4 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut request = vec![];
                loop {
                    let mut buf = [0; 1024];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() > 8192 {
                        break;
                    }
                }
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut reader = HttpRead::open_at(reqwest::Client::new(), url, 9, 4).await?;
            reader.read_exact(&mut [0; 5]).await.map(|_| ())
        })
        .await
        .unwrap();
        assert!(result.is_err(), "{range}, {length}");
        worker.abort();
    }
}

#[tokio::test]
async fn install_signed_out_slot_never_provisions_a_device() {
    let root = scratch("signed-out-store");
    let ctx = Context { root: root.clone(), client: reqwest::Client::new(), auth_helper: PathBuf::from("unused") };
    let id = StoreId(uuid::Uuid::new_v4());
    let error = install(
        &ctx,
        id.clone(),
        BuildSelection::Latest(Edition::Release),
        Arc::new(Progress::default()),
        Cancel::default(),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error.kind, ErrorKind::SignInRequired));
    assert!(!root.join("stores").join(id.0.to_string()).join("provisioning").exists());
    let error = license::unavailable(err(ErrorKind::Network, "fixture"));
    assert!(matches!(error.kind, ErrorKind::LicenseUnavailable));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn operations_can_run_on_tokio_workers() {
    fn send<T: Send>(_: T) {}
    let ctx =
        Context { root: PathBuf::from("unused"), client: reqwest::Client::new(), auth_helper: PathBuf::from("unused") };
    send(sign_in(&ctx, StoreId(uuid::Uuid::new_v4()), Arc::new(Progress::default()), Cancel::default()));
    send(sign_in_with(
        &ctx,
        None,
        Window { login_hint: Some("a@b.c".into()), xbox_codes: 1 },
        Arc::new(Progress::default()),
        Cancel::default(),
    ));
    send(install(
        &ctx,
        StoreId(uuid::Uuid::new_v4()),
        BuildSelection::Latest(Edition::Release),
        Arc::new(Progress::default()),
        Cancel::default(),
    ));
    // checked by the compiler, never run
    let _stage_is_send = |ctx: &Context, build: &InstalledBuild| {
        send(stage(ctx, StoreId(uuid::Uuid::new_v4()), build, Cancel::default()));
        send(stage_into(ctx, StoreId(uuid::Uuid::new_v4()), build, Path::new("/dev/shm/x"), Cancel::default()));
    };
}

#[test]
fn sign_in_frames_may_reach_challenges_but_credentials_only_come_from_login_live() {
    for url in [
        "https://login.live.com/ppsecure/post.srf",
        "https://account.live.com/proofs/Add",
        "https://logincdn.msauth.net/shared/1.0/content/js/x.js",
        "https://iframe.arkoselabs.com/fc/gc/",
        "https://client.hsprotect.net/x",
        "https://privacy.microsoft.com/",
        "about:blank",
    ] {
        assert!(ipc::allowed_navigation(url), "{url}");
    }
    for url in [
        "http://login.live.com/",
        "https://evillive.com/",
        "https://live.com.evil/",
        "https://evil@login.live.com/",
        "https://login.live.com:444/",
        "https://LOGIN.live.com/",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,x",
    ] {
        assert!(!ipc::allowed_navigation(url), "{url}");
    }
    assert!(ipc::credential_page("https://login.live.com/ppsecure/inlineconnect.srf?x=1"));
    assert!(!ipc::credential_page("https://account.live.com/"));
    assert!(!ipc::credential_page("https://login.live.com.evil/"));
    // The helper never forwards the rest of ServerData, which may not even fit in a frame.
    let script = ipc::read_server_data("0123456789abcdef0123456789abcdef").unwrap();
    for field in ipc::DA_FIELDS {
        assert!(script.contains(&format!("{field:?}:ServerData[{field:?}]")));
    }
    assert!(!script.contains("JSON.stringify(ServerData)"));
}

#[tokio::test]
async fn only_registrations_that_never_left_may_be_retried() {
    #[derive(Debug)]
    struct Wrapped(reqwest::Error);
    impl std::fmt::Display for Wrapped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("wrapped")
        }
    }
    impl std::error::Error for Wrapped {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    let refused = reqwest::Client::new().get("http://127.0.0.1:1/").send().await.unwrap_err();
    assert!(device::never_sent(&Wrapped(refused)));
    let server = Server::start().await;
    let url = server.route("/fail", Reply { status: 500, ..Reply::ok(b"fixture".to_vec()) });
    let status = reqwest::Client::new().get(url).send().await.unwrap().error_for_status().unwrap_err();
    assert!(!device::never_sent(&Wrapped(status)), "a server answer may follow a registration");
    assert!(!device::never_sent(&std::io::Error::other("fixture")));
}

#[tokio::test]
async fn unchanged_files_are_hashed_once_and_changed_ones_refetched() {
    let root = scratch("reuse-set");
    let ctx = Context { root: root.clone(), client: reqwest::Client::new(), auth_helper: PathBuf::from("unused") };
    let id =
        BuildId { edition: Edition::Release, version: "1.2.3.4".into(), package_id: uuid::Uuid::new_v4().to_string() };
    let build_root = root.join("builds/release/1.2.3.4").join(&id.package_id);
    let content = build_root.join("content");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join(download::METADATA), b"synthetic-metadata-placeholder").unwrap();
    let mut entries = std::collections::BTreeMap::new();
    let mut segments = std::collections::HashMap::new();
    for (i, name) in ["kept.bin", "changed.bin", "hashless.bin"].into_iter().enumerate() {
        std::fs::write(content.join(name), b"fixture").unwrap();
        let hashes = if name == "hashless.bin" { vec![] } else { vec![[i as u8 + 1; 20]] };
        entries.insert(
            name.to_string(),
            download::Entry {
                offset: i as u64 * 4096,
                length: 7,
                hashes: hashes.clone(),
                encrypted: false,
                disk_sha256: download::disk_hash(&content.join(name)).await.unwrap(),
                local_length: 7,
                patch_revision: None,
            },
        );
        let data_hashs = if name == "changed.bin" { vec![[9; 20]] } else { hashes };
        segments.insert(
            name.to_string(),
            SegmentFile { offset: i as u64 * 4096, length: 7, data_hashs, keep_encrypted: false },
        );
    }
    let receipt = download::Receipt {
        schema: 1,
        adapter: 1,
        build: id.clone(),
        content_id: catalog::content_id(Edition::Release).into(),
        files: entries,
    };
    std::fs::write(build_root.join("install.json"), serde_json::to_vec(&receipt).unwrap()).unwrap();
    let old = download::load(&ctx, &id).await.unwrap();
    let reuse = download::reusable_files(Some(&old), &segments, &Cancel::default()).await.unwrap();
    assert_eq!(reuse, ["kept.bin".to_string()].into());
    assert!(download::reusable_files(None, &segments, &Cancel::default()).await.unwrap().is_empty());
    let cancel = Cancel::default();
    cancel.cancel();
    assert!(matches!(
        download::reusable_files(Some(&old), &segments, &cancel).await.unwrap_err().kind,
        ErrorKind::Cancelled
    ));
    drop(old);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_package_version_id_is_a_safe_folder_name_not_necessarily_a_uuid() {
    use super::download::valid_package_id;
    assert!(valid_package_id("7792d9ce-355a-493c-afbd-768f4a77c3b0"));
    assert!(valid_package_id(&"a1b2c3d4".repeat(6)), "the 48-character IDs the catalogue gives now");
    for bad in ["", "..", ".hidden", "a/b", "a\\b", "a b", &"x".repeat(65)] {
        assert!(!valid_package_id(bad), "{bad:?}");
    }
}

#[test]
fn a_catalogue_version_is_four_numbers_with_or_without_the_package_id_after_them() {
    use super::catalog::numeric_version;
    assert_eq!(numeric_version("1.26.6005.0.ea989132-19db-44c9-98a4-949e064552c7").as_deref(), Some("1.26.6005.0"));
    assert_eq!(numeric_version("1.21.4402.0").as_deref(), Some("1.21.4402.0"));
    for bad in ["1.26.6005", "1.26.x.0", "1.26.6005.0.not-a-uuid", "", "1..2.3"] {
        assert_eq!(numeric_version(bad), None, "{bad:?}");
    }
}
