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

lockstep will also look into Cargo.lock to check for outdated revisions there.

if nothing is required, lockstep won't print anything.

# TODO

opte support is missing
*/

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::hash;
use std::hash::Hash;
use std::path::Path;
use url::Url;

use anyhow::{anyhow, bail, Result};
use cargo_lock::package::SourceKind;
use cargo_toml::Manifest;
use glob::glob;
use reqwest::blocking::Client;

use omicron_zone_package::config::*;
use omicron_zone_package::package::*;

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

    let cargo_path = format!("./{}/Cargo.toml", sub_directory);
    for (dep_key, dep) in &cargo_manifest.dependencies {
        if let Some(detail) = dep.detail() {
            // TODO currently does not check for crates.io, just git
            if let Some(git) = &detail.git {
                if git.ends_with(package) {
                    if let Some(rev) = &detail.rev {
                        if rev != ensure_rev {
                            println!(
                                "update {:?} {:?} rev from {} to {}",
                                cargo_path, dep_key, rev, ensure_rev,
                            );
                            update_required = true;
                        }
                    }
                }
            }
        }
    }

    if let Some(workspace) = &cargo_manifest.workspace {
        for (dep_key, dep) in &workspace.dependencies {
            if let Some(detail) = dep.detail() {
                // TODO currently does not check for crates.io, just git
                if let Some(git) = &detail.git {
                    if git.ends_with(package) {
                        if let Some(rev) = &detail.rev {
                            if rev != ensure_rev {
                                println!(
                                    "update {:?} {:?} rev from {} to {}",
                                    cargo_path, dep_key, rev, ensure_rev,
                                );
                                update_required = true;
                            }
                        }
                    }
                }
            }
        }

        for member in &workspace.members {
            // use glob to support members that look like "lib/*"
            let path = format!("./{}/{}/Cargo.toml", sub_directory, member);
            for sub_cargo_path in glob(&path).expect("failed to glob pattern") {
                let sub_cargo_path = match sub_cargo_path {
                    Ok(path) => path,
                    Err(e) => {
                        return Err(anyhow!(e));
                    }
                };

                let sub_cargo_manifest = match Manifest::from_path(&sub_cargo_path) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("error with {:?}: {}", &sub_cargo_path, e);
                        continue;
                    }
                };

                update_required |= compare_cargo_toml_revisions(
                    // get directory name for resolved Cargo.toml path
                    sub_cargo_path.parent().unwrap().to_str().unwrap(),
                    &sub_cargo_manifest,
                    package,
                    ensure_rev,
                )?;
            }
        }
    }

    Ok(update_required)
}

/// Return all the dependencies named all Cargo.toml files
fn get_explicit_dependencies(
    sub_directory: &str,
    dependencies: &mut HashSet<String>,
) -> Result<()> {
    let cargo_path = format!("./{}/Cargo.toml", sub_directory);
    let cargo_manifest = match Manifest::from_path(&cargo_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error with {:?}: {}", &cargo_path, e);
            return Ok(());
        }
    };

    for dep_key in cargo_manifest.dependencies.keys() {
        dependencies.insert(dep_key.clone());
    }

    if let Some(workspace) = &cargo_manifest.workspace {
        for dep_key in workspace.dependencies.keys() {
            dependencies.insert(dep_key.clone());
        }

        for member in &workspace.members {
            // use glob to support members that look like "lib/*"
            let path = format!("./{}/{}/Cargo.toml", sub_directory, member);
            for sub_cargo_path in glob(&path).expect("failed to glob pattern") {
                let sub_cargo_path = match sub_cargo_path {
                    Ok(path) => path,
                    Err(e) => {
                        return Err(anyhow!(e));
                    }
                };

                get_explicit_dependencies(
                    sub_cargo_path.parent().unwrap().to_str().unwrap(),
                    dependencies,
                )?;
            }
        }
    }

    Ok(())
}

// Implement SourceID like rust-lang/Cargo, not like in rustsec/rustsec (read:
// with manual impls)
#[derive(Clone, Debug, Eq)]
struct MySourceId {
    pub url: Url,
    pub kind: SourceKind,
    pub precise: Option<String>,
}

impl PartialEq for MySourceId {
    fn eq(&self, other: &MySourceId) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl PartialOrd for MySourceId {
    fn partial_cmp(&self, other: &MySourceId) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MySourceId {
    fn cmp(&self, other: &MySourceId) -> Ordering {
        // Ignore precise and name
        match self.kind.cmp(&other.kind) {
            Ordering::Equal => {}
            other => return other,
        };

        self.url.cmp(&other.url)
    }
}

impl Hash for MySourceId {
    fn hash<S: hash::Hasher>(&self, into: &mut S) {
        self.url.hash(into);
        self.kind.hash(into);
    }
}

fn check_cargo_lock_revisions(
    sub_directory: &str,
    latest_revs: &BTreeMap<String, String>,
) -> Result<()> {
    use cargo_lock::package;
    use cargo_lock::Lockfile;

    let cargo_lockfile = Lockfile::load(format!("{}/Cargo.lock", sub_directory))?;
    let dependencies = {
        let mut dependencies = HashSet::new();
        get_explicit_dependencies(sub_directory, &mut dependencies)?;
        dependencies
    };

    // Check for conflicting SourceId
    let mut sources: HashSet<MySourceId> = HashSet::new();

    for package in &cargo_lockfile.packages {
        if let Some(source) = &package.source {
            let path = source.url().path();
            if let Some(repo) = path.strip_prefix("/oxidecomputer/") {
                if let package::SourceKind::Git(reference) = source.kind() {
                    if matches!(reference, package::GitReference::Branch(..)) {
                        if let Some(precise) = source.precise() {
                            // println!("checking {}/Cargo.lock {} {}", sub_directory, repo, precise);

                            if let Some(latest_rev) = latest_revs.get(repo) {
                                if latest_rev != precise {
                                    // only suggest running `cargo update -p`
                                    // for packages in a Cargo.toml
                                    if dependencies.contains(&package.name.to_string()) {
                                        println!(
                                            "{}/Cargo.lock has old rev for {} {}! update {} to {}",
                                            sub_directory, repo, package.name, precise, latest_rev,
                                        );
                                    }
                                }
                            } else {
                                // println!("no latest rev for {}", repo);
                            }
                        }
                    }
                }
            }

            let my_source_id = MySourceId {
                url: source.url().clone(),
                kind: source.kind().clone(),
                precise: source.precise().map(|x| x.to_string()).clone(),
            };

            if let Some(existing_source) = sources.get(&my_source_id) {
                if existing_source.precise == my_source_id.precise {
                    sources.insert(my_source_id);
                } else {
                    panic!(
                        "{}/Cargo.lock has a mismatch for {:?} != {:?}!",
                        sub_directory, existing_source, my_source_id,
                    );
                }
            } else {
                sources.insert(my_source_id);
            }
        }
    }

    for source in sources {
        println!("{}/Cargo.lock has source {:?}", sub_directory, source);
    }

    Ok(())
}

fn main() -> Result<()> {
    let client = Client::new();
    let mut update_required = false;

    // Check we're in a location that contains checkouts of relevant repos
    for repo in &["crucible", "propolis", "omicron", "maghemite", "dendrite"] {
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

    let maghemite_repo = git2::Repository::open("maghemite")?;
    let maghemite_rev: git2::Oid = maghemite_repo.head()?.target().unwrap();

    latest_revs.insert("maghemite".to_string(), maghemite_rev.to_string());

    let dendrite_repo = git2::Repository::open("dendrite")?;
    let dendrite_rev: git2::Oid = dendrite_repo.head()?.target().unwrap();

    latest_revs.insert("dendrite".to_string(), dendrite_rev.to_string());

    let omicron_repo = git2::Repository::open("omicron")?;
    let omicron_rev: git2::Oid = omicron_repo.head()?.target().unwrap();

    latest_revs.insert("omicron".to_string(), omicron_rev.to_string());

    // Check the revs in crucible's Cargo.lock
    check_cargo_lock_revisions("crucible", &latest_revs)?;

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

    // Check the revs in propolis' Cargo.lock
    check_cargo_lock_revisions("propolis", &latest_revs)?;

    // Check if omicron needs to:
    // - update crucible cargo revs
    // - update propolis cargo revs

    check_cargo_lock_revisions("omicron", &latest_revs)?;

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

    for (name, package) in &package_manifest.packages {
        if let PackageSource::Prebuilt {
            repo,
            commit,
            sha256,
        } = &package.source
        {
            if !latest_revs.contains_key(&repo.clone()) {
                println!("no latest rev for {}", repo);
                continue;
            }

            // skip checking maghemite for now
            if repo == &"maghemite".to_string() {
                continue;
            }

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
                continue;
            }

            let response = response.unwrap();

            if !response.status().is_success() {
                println!(
                    "wait for {} image for {} to be built (reqwest returned {})",
                    name,
                    propolis_rev,
                    response.status(),
                );
                continue;
            }

            let response_hash = response.text()?;
            if response_hash.trim() != sha256 {
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
