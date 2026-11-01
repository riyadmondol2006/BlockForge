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
    assert!(ipc::decode(br#"{"version":2,"message":{"type":"Complete"}}"#).is_err());
    assert!(ipc::decode(br#"{"version":1,"message":{"type":"Unknown"}}"#).is_err());
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
    let script = ipc::read_server_data();
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
