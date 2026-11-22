use std::collections::VecDeque;
use std::future::Future;
use std::sync::Mutex;

use serde_json::{json, Value};

use super::cache::{self, Cached};
use super::client::{self, CatalogClient, Fetched, Transport};
use super::types::*;
use super::*;
use crate::error::{AppError, AppResult};
use crate::layers::plan::{self as layers_plan, PlanRequest, PlanSource};
use crate::library::db::LibraryDb;

const CATALOG: &str = include_str!("../../../../src/lib/api/fixtures/catalog/catalog.json");
const VERSIONS: &str = include_str!("../../../../src/lib/api/fixtures/catalog/versions.json");
const LOADER_VERSIONS: &str =
    include_str!("../../../../src/lib/api/fixtures/catalog/loader-versions.json");
const INSTALL_PLAN: &str =
    include_str!("../../../../src/lib/api/fixtures/catalog/install-plan.json");
const RUNTIME: &str = include_str!("../../../../src/lib/api/fixtures/catalog/runtime.json");
const WIRE_CATALOG: &str =
    include_str!("../../../../src/lib/api/fixtures/catalog/wire/catalog.json");
const WIRE_LOADER_VERSIONS: &str =
    include_str!("../../../../src/lib/api/fixtures/catalog/wire/loader-versions.json");
const WIRE_INSTALL_PLAN: &str =
    include_str!("../../../../src/lib/api/fixtures/catalog/wire/install-plan.json");
const WIRE_RUNTIME: &str =
    include_str!("../../../../src/lib/api/fixtures/catalog/wire/runtime.json");
const WIRE_ERROR: &str = include_str!("../../../../src/lib/api/fixtures/catalog/wire/error.json");

/// Answers from a script, and remembers what it was asked.
#[derive(Default)]
struct Fake {
    answers: Mutex<VecDeque<AppResult<Fetched>>>,
    asked: Mutex<Vec<(String, Option<String>)>>,
}

impl Fake {
    fn with(answers: Vec<AppResult<Fetched>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            asked: Mutex::default(),
        }
    }
    fn asked(&self) -> Vec<(String, Option<String>)> {
        self.asked.lock().unwrap().clone()
    }
}

impl Transport for Fake {
    fn get(
        &self,
        path: &str,
        etag: Option<&str>,
    ) -> impl Future<Output = AppResult<Fetched>> + Send {
        self.asked
            .lock()
            .unwrap()
            .push((path.to_string(), etag.map(str::to_string)));
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(AppError::Network("offline".into())));
        std::future::ready(answer)
    }
}

const T0: i64 = 1_000_000_000_000;
const MINUTE: i64 = 60_000;

fn client<'a>(
    db: &'a LibraryDb,
    fake: &'a Fake,
    ttl: u32,
    now: fn() -> i64,
) -> CatalogClient<'a, Fake> {
    CatalogClient {
        db,
        transport: fake,
        ttl_minutes: ttl,
        now,
    }
}

fn catalog_body() -> Value {
    serde_json::from_str(CATALOG).unwrap()
}

fn body(data: Value, etag: &str) -> AppResult<Fetched> {
    Ok(Fetched::Body {
        etag: Some(etag.into()),
        data,
    })
}

fn seed(db: &LibraryDb, path: &str, fetched_at: i64) {
    cache::write(
        db,
        path,
        &Cached {
            etag: Some("\"v1\"".into()),
            body: CATALOG.to_string(),
            fetched_at,
        },
    )
    .unwrap();
}

// ------------------------------------------------------------ fixtures

#[test]
fn every_w5_fixture_reads_as_its_wire_struct() {
    serde_json::from_str::<CatalogWire>(CATALOG).unwrap();
    serde_json::from_str::<CatalogWire>(WIRE_CATALOG).unwrap();
    serde_json::from_str::<Page<GameVersion>>(VERSIONS).unwrap();
    serde_json::from_str::<LoaderVersionList>(LOADER_VERSIONS).unwrap();
    serde_json::from_str::<LoaderVersionList>(WIRE_LOADER_VERSIONS).unwrap();
    serde_json::from_str::<InstallPlanWire>(INSTALL_PLAN).unwrap();
    serde_json::from_str::<InstallPlanWire>(WIRE_INSTALL_PLAN).unwrap();
    serde_json::from_str::<RuntimeWire>(RUNTIME).unwrap();
    serde_json::from_str::<RuntimeWire>(WIRE_RUNTIME).unwrap();
}

#[test]
fn a_field_the_contract_does_not_have_is_refused() {
    let mut doc = catalog_body();
    doc["loaders"][0]["installer"] = json!("https://elsewhere.example/run.exe");
    assert!(serde_json::from_value::<CatalogWire>(doc).is_err());
}

#[test]
fn the_compiled_in_catalog_reads_as_a_snapshot() {
    for (slug, raw) in &PlanSource::compiled().catalogs {
        let snap: Snapshot = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(&snap.app.slug, slug);
    }
    assert!(fallback::snapshot(PlanSource::compiled(), "minecraft").is_ok());
}

// --------------------------------------------------------------- policy

#[tokio::test]
async fn inside_the_ttl_the_cached_copy_is_served_without_asking() {
    let db = LibraryDb::open_memory().unwrap();
    let path = client::catalog_path(412, &[], 30);
    seed(&db, &path, T0);
    let fake = Fake::default();
    let c = client(&db, &fake, 360, || T0 + 5 * MINUTE);

    let view = catalog(&c, 412, "minecraft", &[], 30, false).await.unwrap();
    assert_eq!(view.provenance.source, Source::Cache);
    assert!(!view.provenance.stale);
    assert_eq!(view.provenance.fetched_at, Some(T0));
    assert!(fake.asked().is_empty());
}

#[tokio::test]
async fn after_the_ttl_a_304_keeps_the_copy_and_restarts_its_age() {
    let db = LibraryDb::open_memory().unwrap();
    let path = client::catalog_path(412, &[], 30);
    seed(&db, &path, T0);
    let fake = Fake::with(vec![Ok(Fetched::NotModified)]);
    let c = client(&db, &fake, 360, || T0 + 361 * MINUTE);

    let view = catalog(&c, 412, "minecraft", &[], 30, false).await.unwrap();
    assert_eq!(view.provenance.source, Source::Cache);
    assert_eq!(view.provenance.fetched_at, Some(T0 + 361 * MINUTE));
    assert_eq!(
        fake.asked(),
        vec![(path.clone(), Some("\"v1\"".to_string()))]
    );
    assert_eq!(
        cache::read(&db, &path).unwrap().unwrap().fetched_at,
        T0 + 361 * MINUTE
    );
}

#[tokio::test]
async fn a_ttl_of_zero_revalidates_every_read() {
    let db = LibraryDb::open_memory().unwrap();
    let path = client::catalog_path(412, &[], 30);
    seed(&db, &path, T0);
    let fake = Fake::with(vec![Ok(Fetched::NotModified)]);
    let c = client(&db, &fake, 0, || T0 + 1);
    catalog(&c, 412, "minecraft", &[], 30, false).await.unwrap();
    assert_eq!(fake.asked().len(), 1);
}

#[tokio::test]
async fn a_200_replaces_the_copy_and_its_etag() {
    let db = LibraryDb::open_memory().unwrap();
    let fake = Fake::with(vec![body(catalog_body(), "\"v2\"")]);
    let c = client(&db, &fake, 360, || T0);

    let view = catalog(&c, 412, "minecraft", &[], 30, false).await.unwrap();
    assert_eq!(view.provenance.source, Source::Remote);
    assert_eq!(view.app.slug, "minecraft");
    let row = cache::read(&db, &client::catalog_path(412, &[], 30))
        .unwrap()
        .unwrap();
    assert_eq!(row.etag.as_deref(), Some("\"v2\""));
    assert_eq!(row.fetched_at, T0);
}

#[tokio::test]
async fn offline_after_the_ttl_serves_the_copy_marked_stale() {
    let db = LibraryDb::open_memory().unwrap();
    let path = client::catalog_path(412, &[], 30);
    seed(&db, &path, T0);
    let fake = Fake::with(vec![Err(AppError::Network("offline".into()))]);
    let c = client(&db, &fake, 360, || T0 + 400 * MINUTE);

    let view = catalog(&c, 412, "minecraft", &[], 30, false).await.unwrap();
    assert_eq!(view.provenance.source, Source::Cache);
    assert!(view.provenance.stale);
    assert_eq!(view.provenance.fetched_at, Some(T0));
}

#[tokio::test]
async fn a_server_error_or_an_unknown_shape_is_treated_as_offline() {
    let db = LibraryDb::open_memory().unwrap();
    let path = client::catalog_path(412, &[], 30);
    seed(&db, &path, T0);
    let fake = Fake::with(vec![
        Ok(Fetched::Refused {
            status: 503,
            code: "unavailable".into(),
            message: "down".into(),
            note: None,
        }),
        body(json!({ "surprise": true }), "\"v9\""),
    ]);
    let c = client(&db, &fake, 0, || T0 + 1);
    for _ in 0..2 {
        let view = catalog(&c, 412, "minecraft", &[], 30, false).await.unwrap();
        assert!(view.provenance.stale);
    }
    // The odd body was not cached over the good one.
    assert_eq!(
        cache::read(&db, &path).unwrap().unwrap().etag.as_deref(),
        Some("\"v1\"")
    );
}

#[tokio::test]
async fn with_nothing_cached_and_no_network_the_compiled_snapshot_is_served() {
    let db = LibraryDb::open_memory().unwrap();
    let fake = Fake::default();
    let c = client(&db, &fake, 360, || T0);

    let view = catalog(&c, 412, "minecraft", &[], 30, false).await.unwrap();
    assert_eq!(view.provenance.source, Source::Compiled);
    assert_eq!(view.provenance.fetched_at, None);
    assert_eq!(view.app.slug, "minecraft");

    let lv = loader_versions(
        &c,
        412,
        "minecraft",
        "forge",
        Some("1.21.4"),
        &[],
        30,
        false,
    )
    .await
    .unwrap();
    assert_eq!(lv.provenance.source, Source::Compiled);
}

#[tokio::test]
async fn with_no_snapshot_either_the_error_says_so() {
    let db = LibraryDb::open_memory().unwrap();
    let fake = Fake::default();
    let c = client(&db, &fake, 360, || T0);
    let err = catalog(&c, 9, "no-such-game", &[], 30, false)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AppError::Network(ref m) if m.contains("no copy")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_withdrawn_answer_is_an_error_carrying_the_note() {
    let envelope: Value = serde_json::from_str(WIRE_ERROR).unwrap();
    let db = LibraryDb::open_memory().unwrap();
    let path = client::catalog_path(412, &[], 30);
    seed(&db, &path, T0);
    let fake = Fake::with(vec![Ok(Fetched::Refused {
        status: 410,
        code: envelope["code"].as_str().unwrap().into(),
        message: envelope["message"].as_str().unwrap().into(),
        note: envelope["note"].as_str().map(str::to_string),
    })]);
    let c = client(&db, &fake, 360, || T0);

    let err = catalog(&c, 412, "minecraft", &[], 30, true)
        .await
        .unwrap_err();
    let AppError::Invalid(msg) = err else {
        panic!("{err:?}")
    };
    assert!(msg.contains(envelope["note"].as_str().unwrap()), "{msg}");
}

#[tokio::test]
async fn not_found_forgets_the_copy_and_falls_back_to_the_snapshot() {
    let db = LibraryDb::open_memory().unwrap();
    let path = client::catalog_path(412, &[], 30);
    seed(&db, &path, T0);
    let fake = Fake::with(vec![Ok(Fetched::Refused {
        status: 404,
        code: "catalog_unmanaged".into(),
        message: "Not managed.".into(),
        note: None,
    })]);
    let c = client(&db, &fake, 360, || T0);

    let view = catalog(&c, 412, "minecraft", &[], 30, true).await.unwrap();
    assert_eq!(view.provenance.source, Source::Compiled);
    assert!(cache::read(&db, &path).unwrap().is_none());
}

#[tokio::test]
async fn a_cached_row_that_no_longer_parses_is_dropped() {
    let db = LibraryDb::open_memory().unwrap();
    let path = client::catalog_path(412, &[], 30);
    cache::write(
        &db,
        &path,
        &Cached {
            etag: Some("\"old\"".into()),
            body: "{\"from\":\"an older app\"}".into(),
            fetched_at: T0,
        },
    )
    .unwrap();
    let fake = Fake::with(vec![body(catalog_body(), "\"v2\"")]);
    let c = client(&db, &fake, 360, || T0 + 1);
    let view = catalog(&c, 412, "minecraft", &[], 30, false).await.unwrap();
    assert_eq!(view.provenance.source, Source::Remote);
    // Asked without the dropped row's ETag.
    assert_eq!(fake.asked()[0].1, None);
}

// ---------------------------------------------------------------- paths

#[test]
fn paths_carry_the_limit_channels_and_encoded_values() {
    assert_eq!(
        client::catalog_path(412, &[], 0),
        "/apps/412/catalog?limit=0"
    );
    assert_eq!(
        client::catalog_path(412, &[Channel::Preview, Channel::Stable], 30),
        "/apps/412/catalog?limit=30&channels=STABLE%2CPREVIEW"
    );
    assert_eq!(
        client::loader_versions_path(412, "forge", Some("1.21.4 pre 1"), &[Channel::Stable], 5)
            .unwrap(),
        "/apps/412/loaders/forge/versions?gameVersion=1.21.4+pre+1&limit=5"
    );
    assert!(client::loader_versions_path(412, "../auth", None, &[], 5).is_err());
    assert!(client::loader_versions_path(412, "a/b", None, &[], 5).is_err());
    assert_eq!(
        client::install_plan_path(412, "1.21.4", Some("fabric"), Some("0.16.9"), "linux-x64", "client"),
        "/apps/412/install-plan?gameVersion=1.21.4&loader=fabric&loaderVersion=0.16.9&platform=linux-x64&side=client"
    );
}

// ----------------------------------------------------------------- plans

fn fixture_request(wire: &InstallPlanWire) -> PlanRequest {
    PlanRequest {
        app_slug: wire.app.slug.clone(),
        game_version: wire.request.game_version.clone(),
        loader: wire.request.loader.clone(),
        loader_version: wire.request.loader_version.clone(),
        side: wire.request.side.clone(),
        platform: wire.request.platform.clone(),
        sandbox_id: "sb-1".into(),
        sandbox_name: "Test".into(),
        target_dir: "/tmp/sb-1".into(),
    }
}

#[test]
fn the_fixture_plan_resolves_through_the_same_resolver() {
    let wire: InstallPlanWire = serde_json::from_str(INSTALL_PLAN).unwrap();
    let req = fixture_request(&wire);
    let source = wire_plan::plan_source(&wire, &req, &PlanSource::compiled().keys).unwrap();
    let plan = layers_plan::resolve(&source, &req)
        .unwrap()
        .expect("a plan");
    let keys: Vec<&str> = plan
        .layers
        .iter()
        .map(|l| l.manifest_key.as_str())
        .collect();
    let want: Vec<&str> = wire
        .layers
        .iter()
        .map(|l| l.manifest_key.as_str())
        .collect();
    assert_eq!(keys, want);
    assert_eq!(plan.trust, layers_plan::Trust::Official);
}

#[test]
fn a_plan_for_another_request_or_with_a_tampered_layer_is_refused() {
    let wire: InstallPlanWire = serde_json::from_str(INSTALL_PLAN).unwrap();
    let keys = &PlanSource::compiled().keys;

    let mut other = fixture_request(&wire);
    other.game_version = "1.20.1".into();
    assert!(wire_plan::plan_source(&wire, &other, keys).is_err());

    let mut tampered = wire.clone();
    tampered.layers[0]
        .manifest
        .insert("title".into(), json!("Something else"));
    assert!(wire_plan::plan_source(&tampered, &fixture_request(&wire), keys).is_err());
}

#[tokio::test]
async fn the_plan_source_comes_through_the_cache_policy() {
    let db = LibraryDb::open_memory().unwrap();
    let wire: Value = serde_json::from_str(INSTALL_PLAN).unwrap();
    let parsed: InstallPlanWire = serde_json::from_value(wire.clone()).unwrap();
    let req = fixture_request(&parsed);
    let fake = Fake::with(vec![body(wire, "\"p1\"")]);
    let c = client(&db, &fake, 360, || T0);

    let (source, prov) = plan_source(&c, 412, &req).await.unwrap().expect("served");
    assert_eq!(prov.source, Source::Remote);
    assert!(source.catalogs.contains_key("minecraft"));
    // The second read inside the TTL is the cached plan, so preview and
    // consent see the same hash.
    let (_, again) = plan_source(&c, 412, &req).await.unwrap().expect("cached");
    assert_eq!(again.source, Source::Cache);
    assert_eq!(fake.asked().len(), 1);
}
