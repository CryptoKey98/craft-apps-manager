//! Authenticated broker loop for reversible macOS bundle publication.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Status {
    pub next: usize,
    pub pending: Option<usize>,
    pub installed_hash: String,
}

pub(super) fn run(
    manifest: Manifest,
    catalog: VerifiedCatalog,
    mut journey: Option<Journey>,
    copies: Vec<PathBuf>,
    trust: &TrustContext,
) -> Result<()> {
    let packages = grant_packages(
        &catalog,
        &manifest.source,
        &manifest.offer,
        manifest.grant_span,
    )?;
    let target_bundle = crate::self_update::app_bundle(&manifest.target)?;
    let mut bundle = super::macos_bundle::RootBundleTransaction::open(
        &manifest.target,
        manifest.account.parse()?,
    )?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let mut parent = connection(manifest.callback)?;
    let hello: Hello = wire_read(&mut parent, &manifest.secret)?;
    ensure!(hello.role == "native-parent", "Wrong approval parent");
    wire_send(
        &mut parent,
        &manifest.secret,
        &Hello {
            role: "native-worker".into(),
            port: listener.local_addr()?.port(),
        },
    )?;
    drop(parent);
    let mut gate = Gate {
        token: "session".into(),
        expires: manifest.expires,
        count: copies.len(),
        next: 0,
        pending: None,
    };
    let mut installed_hash = manifest.source_hash.clone();
    let mut prior = super::super::trust::TrustRecord::from_catalog(&catalog);
    let mut idle = Instant::now();
    while now() < manifest.expires && idle.elapsed() < IDLE {
        let mut stream = match accepted_connection(&listener) {
            Ok(stream) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let Ok(req) = wire_read::<Request>(&mut stream, &manifest.secret) else {
            continue;
        };
        if req.role != "native-client" {
            continue;
        }
        let mut reply = Reply {
            role: "native-result".into(),
            id: req.id.clone(),
            code: None,
            error: None,
            log: String::new(),
        };
        let operation = (|| -> Result<bool> {
            ensure!(now() < manifest.expires, "Administrator approval expired");
            match req.action.as_str() {
                "close" => {
                    gate.close("session", now())?;
                    reply.code = Some(0);
                    return Ok(true);
                }
                "abort" => {
                    bundle.rollback_pending()?;
                    reply.code = Some(0);
                    return Ok(true);
                }
                "status" => {
                    ensure!(
                        files::hash(&crate::self_update::staged_executable(&target_bundle))?
                            == installed_hash,
                        "Installed manager changed"
                    );
                    reply.log = serde_json::to_string(&Status {
                        next: gate.next,
                        pending: gate.pending,
                        installed_hash: installed_hash.clone(),
                    })?;
                    reply.code = Some(0);
                    return Ok(false);
                }
                "ack" => {
                    ensure!(
                        gate.pending == Some(req.index),
                        "Wrong or repeated acknowledgement"
                    );
                    bundle.verify(req.index)?;
                    // Validate the gate before consuming the recoverable transaction.
                    let acknowledged_at = now();
                    ensure!(acknowledged_at < gate.expires, "Acknowledgement expired");
                    reply.log = bundle.acknowledge(req.index)?.join("\n");
                    reply.code = Some(0);
                    gate.acknowledge("session", req.index, acknowledged_at)?;
                    // Keep read-only status available until the client confirms receipt.
                    return Ok(false);
                }
                "install" => {}
                _ => anyhow::bail!("Unknown macOS approval action"),
            }
            let index = req.index;
            ensure!(
                index < packages.len() && gate.pending.is_none(),
                "Unapproved bundle index"
            );
            let profile = req.profile.as_ref().context("Missing source profile")?;
            let fresh = verify_catalog(&req.envelope, trust, now(), Some(&prior))?;
            let selection = SignedSelection {
                catalog: fresh.clone(),
                offer: req.offer.clone().context("Missing signed offer")?,
            };
            if index == 0 {
                if let Some(journey) = &journey {
                    journey.validate_selection(&selection, profile, trust, now())?;
                } else {
                    validate_direct(&manifest, &catalog, &selection, profile)?;
                }
            } else {
                journey = Some(journey.as_ref().context("No approved bridge")?.next(
                    &selection,
                    profile,
                    trust,
                    now(),
                )?);
            }
            ensure!(
                selection.offer.immediate_package_id == packages[index].id,
                "Bundle order changed"
            );
            ensure!(
                files::hash(&crate::self_update::staged_executable(&target_bundle))?
                    == installed_hash,
                "Bundle source changed"
            );
            gate.claim("session", index, now())?;
            // Root stages only the private, fully revalidated signed archive.
            bundle.stage(index, &copies[index], packages[index], &fresh)?;
            ensure!(now() < manifest.expires, "Consent expired during staging");
            installed_hash = bundle.publish(index, &installed_hash)?;
            prior = super::super::trust::TrustRecord::from_catalog(&fresh);
            reply.code = Some(0);
            Ok(false)
        })();
        let finished = match operation {
            Ok(value) => value,
            Err(e) => {
                let rollback = bundle.rollback_pending();
                reply.error = Some(match rollback {
                    Ok(()) => format!("{e:#}"),
                    Err(recovery) => format!("{e:#}. Recovery retained: {recovery:#}"),
                });
                true
            }
        };
        let sent = wire_send(&mut stream, &manifest.secret, &reply);
        if req.action != "status" {
            idle = Instant::now();
        }
        if finished {
            return sent;
        }
        // A lost install response does not repeat publication; only status/ack/abort remain.
    }
    bundle.rollback_pending()
}

fn validate_direct(
    manifest: &Manifest,
    original: &VerifiedCatalog,
    selection: &SignedSelection,
    profile: &ClientProfile,
) -> Result<()> {
    super::super::transaction::validate_offer(&selection.catalog, profile, &selection.offer)?;
    ensure!(
        serde_json::to_value(profile)? == serde_json::to_value(&manifest.source)?
            && selection.offer == manifest.offer,
        "Direct source scope changed"
    );
    let id = &selection.offer.immediate_package_id;
    let old = original
        .catalog()
        .packages
        .iter()
        .find(|p| &p.id == id)
        .context("Missing approved package")?;
    let new = selection
        .catalog
        .catalog()
        .packages
        .iter()
        .find(|p| &p.id == id)
        .context("Missing fresh package")?;
    ensure!(
        serde_json::to_value(old)? == serde_json::to_value(new)?
            && serde_json::to_value(original.catalog().release(old))?
                == serde_json::to_value(selection.catalog.catalog().release(new))?,
        "Direct signed package changed"
    );
    Ok(())
}
