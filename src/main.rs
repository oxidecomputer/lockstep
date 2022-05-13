// Copyright 2022 Oxide Computer Company
/*!
This tool will print out the steps necessary to keep the crucible, propolis,
and omicron repositories using the same revisions. Specifically, it will:

- look into Cargo.toml for each project, and print out the changes needed to use
  the same git revisions
- look into omicron's package-manifest.toml, and print out the changes needed to
  use the correct git revision and the expected image digest.

lockstep reads information from the local filesystem, not from git remotes, and
as a result requires checking out all of the repositories into the same
directory and running the tool from that directory.

for example, if propolis was updated, lockstep will print something like:
```text
update ./omicron/sled-agent/Cargo.toml propolis rev from ec4f3a41a638ea6c3316a86f30f1895f4877f2ef to eaec980e060b368c4ca39aaaaf7757cecdb43ecc
```

another example: all of the Cargo.toml values are correct but omicron's
package-manifest.toml needs updating:
```text
update omicron package manifest crucible sha256 from 9f73687e4d883a7277af6655e77026188144ada144e4243c90cc139a9a9df6d7 to 174856320e151aeeb12c595392c2289934a0345f669126297cce9ca7249099e3
update omicron package manifest crucible rev from 257032d1e842901d427f344a396d78b9b85b183f to cb363bcb1976093437be33d0160667cd89e53611
wait for propolis-server image for 47ef18a5b0eb7a208ae43e669cf0a93d65576114 to be built (reqwest returned 500 Internal Server Error)
```

if nothing is required, lockstep won't print anything.

# TODO

opte support is missing
*/

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Result};
use cargo_toml::Manifest;
use reqwest::blocking::Client;

use omicron_package::{Config, ExternalPackageSource};

/// Recursively search each Cargo.toml to see if a package's revision needs
/// updating. Print out an instruction if it does, and return if an update is
/// required.
fn compare_cargo_toml_revisions(
    sub_directory: &str,
    cargo_manifest: &Manifest,
    package: &str,
    ensure_rev: &str,
) -> Result<bool> {
    let mut update_required = false;

    if let Some(workspace) = &cargo_manifest.workspace {
        for member in &workspace.members {
            let sub_cargo_path = format!("./{}/{}/Cargo.toml", sub_directory, member);
            let sub_cargo_manifest = Manifest::from_path(&sub_cargo_path)?;

            update_required |= compare_cargo_toml_revisions(
                &format!("./{}/{}", sub_directory, member),
                &sub_cargo_manifest,
                package,
                ensure_rev,
            )?;

            for dep in sub_cargo_manifest.dependencies.values() {
                if let Some(detail) = dep.detail() {
                    // TODO currently does not check for crates.io, just git
                    if let Some(git) = &detail.git {
                        if git.contains(package) {
                            if let Some(rev) = &detail.rev {
                                if rev != ensure_rev {
                                    println!(
                                        "update {} {} rev from {} to {}",
                                        sub_cargo_path, package, rev, ensure_rev,
                                    );
                                    update_required = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(update_required)
}

fn main() -> Result<()> {
    let client = Client::new();
    let mut update_required = false;

    // Check we're in a location that contains checkouts of relevant repos
    for repo in &["crucible", "propolis", "omicron"] {
        if !Path::new(&repo).exists() {
            bail!("cannot find your local checkout of {}!", repo);
        }
    }

    // The latest revisions of repos
    let mut latest_revs: BTreeMap<String, String> = BTreeMap::default();

    // Get the crucible and propolis revisions in the checked out directories
    let crucible_repo = git2::Repository::open("crucible")?;
    let crucible_rev: git2::Oid = crucible_repo.head()?.target().unwrap();

    latest_revs.insert("crucible".to_string(), crucible_rev.to_string());

    let propolis_repo = git2::Repository::open("propolis")?;
    let propolis_rev: git2::Oid = propolis_repo.head()?.target().unwrap();

    latest_revs.insert("propolis".to_string(), propolis_rev.to_string());

    // Ensure propolis uses this crucible revision
    update_required |= compare_cargo_toml_revisions(
        "propolis",
        &Manifest::from_path("./propolis/Cargo.toml")?,
        "crucible",
        &crucible_rev.to_string(),
    )?;

    if update_required {
        return Ok(());
    }

    // Check if omicron needs to:
    // - update crucible cargo revs
    // - update propolis cargo revs

    update_required |= compare_cargo_toml_revisions(
        "omicron",
        &Manifest::from_path("./omicron/Cargo.toml")?,
        "crucible",
        &crucible_rev.to_string(),
    )?;

    update_required |= compare_cargo_toml_revisions(
        "omicron",
        &Manifest::from_path("./omicron/Cargo.toml")?,
        "propolis",
        &propolis_rev.to_string(),
    )?;

    if update_required {
        return Ok(());
    }

    // Check if omicron needs to update package-manifest for new crucible and propolis images
    let package_manifest: Config =
        toml::from_str(&std::fs::read_to_string("./omicron/package-manifest.toml")?)?;

    for (name, package) in &package_manifest.external_packages {
        if let ExternalPackageSource::Prebuilt {
            repo,
            commit,
            sha256,
        } = &package.source
        {
            // make sure images are built
            let response = client
                    .get(&format!("
                        https://buildomat.eng.oxide.computer/public/file/oxidecomputer/{}/image/{}/{}.sha256.txt",
                        repo,
                        latest_revs[&repo.clone()],
                        name))
                    .send();

            if let Err(e) = response {
                println!(
                    "wait for {} image for {} to be built (reqwest returned {})",
                    name, propolis_rev, e,
                );
                return Ok(());
            }

            let response = response.unwrap();

            if !response.status().is_success() {
                println!(
                    "wait for {} image for {} to be built (reqwest returned {})",
                    name,
                    propolis_rev,
                    response.status(),
                );
                return Ok(());
            }

            let response_hash = response.text()?;
            if response_hash.trim() != *sha256 {
                println!(
                    "update omicron package manifest {} sha256 from {} to {}",
                    name,
                    sha256,
                    response_hash.trim()
                );
                update_required = true;
            }

            // make sure rev is up to date
            if *commit != latest_revs[&repo.clone()] {
                println!(
                    "update omicron package manifest {} rev from {} to {}",
                    name,
                    commit,
                    latest_revs[&repo.clone()]
                );
                update_required = true;
            }
        }
    }

    if update_required {
        return Ok(());
    }

    Ok(())
}
