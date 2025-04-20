use super::v0_to_v1;
use crate::database::storage::storage_schema::traits::StorageWriter;
use crate::database::storage::storage_schema::SimpleRustyStorage;

// migrates a wallet db from a lower schema version to a higher version.
//
// this fn should be modified each time that SCHEMA_VERSION is incremented.
//
// there are two types of supported schema modifications:
//  1. a new type is added.
//  2. an existing type is modified (including any sub-type)
//
//  type (1) should typically not require a custom DB migration
//  type (2) may (likely) require a custom DB migration.
//
// let's say we are incrementing to SCHEMA_VERSION 11.
//
// For type 1:
//  in the match statement, add:
//     v if v == 11 => {}
//  this indicates we know about the version, but are not migrating anything.
//
// For type 2:
//  in the match statement, add:
//     v if v == 11 => {
//         log_apply_version(v);
//         v10_to_v11::migrate(storage).await?
//     }
//
//  also, create a module v10_to_v11 and implement the migrate() fn.
pub(crate) async fn migrate_range(
    storage: &mut SimpleRustyStorage,
    version_from: u16,
    version_to: u16,
) -> anyhow::Result<()> {
    assert!(version_from < version_to);

    tracing::info!(
        "wallet database is at schema version: v{}.  migrating to version: v{}",
        version_from,
        version_to
    );

    let log_apply_version = |version| {
        tracing::info!(
            "db migration. applying updates from v{} to v{}",
            version - 1,
            version
        )
    };

    // iterate schema versions and apply migrations, if available.
    // note that every schema version in the range must be known
    // (in the match) else a panic results.  This prevents incrementing
    // the schema version and accidentally forgetting to update this fn.
    for i in version_from..version_to {
        let apply_version = i + 1;

        match apply_version {
            v if v == 1 => {
                log_apply_version(v);
                v0_to_v1::migrate(storage).await?
            }
            _ => panic!("schema version {} is unknown", i),
        }
        storage.persist().await;
    }
    Ok(())
}
