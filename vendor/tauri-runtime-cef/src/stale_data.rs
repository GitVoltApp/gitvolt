// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! GitVolt patch: keeps the root cache path free of Chromium data nothing reads.
//!
//! Three kinds pile up there, and nobody ever looks: profile directories (`Profile-<hash>`, one
//! per webview data directory) of a data directory the application no longer uses, which a
//! rename of the application leaves behind whole; what Chromium's component updater downloaded
//! before the application turned it and the network off; and the log files, which Chromium
//! appends to on every run. [`tidy`] runs at every start, before `cef::initialize`: one
//! directory listing of the root, a stat for the current profile and one per log file, so a
//! tidy root costs well under a millisecond. It only ever removes those names directly under
//! the root, never the current profile or `Default`.
//!
//! Nothing inside the current profile is trimmed from outside: Chromium caps its disk caches
//! (HTTP, code, GPU) itself and empties `blob_storage` at startup, and the rest (`Local Storage`,
//! `Session Storage`, `DIPS`) holds only what the application's own origin keeps.

use std::{fs, path::Path};

/// The prefix of a profile directory the runtime derives from a webview's data directory
/// (`cef_impl::request_context::request_context_cache_path`).
const PROFILE_PREFIX: &str = "Profile-";

/// Chromium's global profile, which webviews without a data directory use.
const DEFAULT_PROFILE: &str = "Default";

/// What the component updater downloaded into the root cache path before the application
/// turned component updates off (`Cef::component_updates`) and kept Chromium off the network.
/// Nothing loads them with the updater off, and nothing would bring a newer copy.
const STALE_COMPONENTS: &[&str] = &[
  "ActorSafetyLists",
  "CertificateRevocation",
  "OnDeviceHeadSuggestModel",
  "OptimizationHints",
  "WasmTtsEngine",
  "WidevineCdm",
  "ZxcvbnData",
  "hyphen-data",
];

/// The log files Chromium appends to in the root cache path: the runtime's default
/// `Cef::log_file`, and Chromium's own name for it, which older versions wrote.
const LOGS: &[&str] = &["cef.log", "chrome_debug.log"];

/// A log over this size is deleted before Chromium opens it again.
const LOG_LIMIT: u64 = 1024 * 1024;

/// What [`tidy`] removed, in name order, and the bytes that freed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Tidied {
  pub removed: Vec<String>,
  pub freed: u64,
}

/// Removes the stale data directly under `root`.
///
/// `current_profile` is the profile directory the application's webviews use, or `None` for
/// `Default`. A profile under another name is only removed once the current one exists, which
/// keeps a wrongly computed name from costing the user the profile in use. Must run in the
/// browser process before `cef::initialize`.
pub(crate) fn tidy(root: &Path, current_profile: Option<&Path>) -> Tidied {
  let current = match current_profile {
    Some(path) => top_level_name(root, path),
    None => Some(DEFAULT_PROFILE.to_string()),
  }
  .filter(|name| root.join(name).is_dir());
  let mut stale_profiles = Vec::new();
  let mut stale = Vec::new();
  let Ok(entries) = fs::read_dir(root) else {
    return Tidied::default();
  };
  for entry in entries.flatten() {
    let Ok(name) = entry.file_name().into_string() else {
      continue;
    };
    // The entry's own type, which on Linux comes with the listing: a symbolic link is neither.
    let Ok(file_type) = entry.file_type() else {
      continue;
    };
    if file_type.is_dir() {
      if STALE_COMPONENTS.contains(&name.as_str()) {
        stale.push(name);
      } else if name.starts_with(PROFILE_PREFIX) {
        stale_profiles.push(name);
      }
    } else if file_type.is_file()
      && LOGS.contains(&name.as_str())
      && entry.metadata().is_ok_and(|m| m.len() > LOG_LIMIT)
    {
      stale.push(name);
    }
  }
  if let Some(current) = &current {
    stale.extend(stale_profiles.into_iter().filter(|name| name != current));
  }
  stale.sort();

  let mut tidied = Tidied::default();
  for name in stale {
    let path = root.join(&name);
    let size = size_of(&path);
    let gone = if path.is_dir() {
      fs::remove_dir_all(&path)
    } else {
      fs::remove_file(&path)
    };
    match gone {
      Ok(()) => {
        tidied.freed += size;
        tidied.removed.push(name);
      }
      Err(e) => log::warn!("couldn't remove stale Chromium data {}: {e}", path.display()),
    }
  }
  if !tidied.removed.is_empty() {
    log::info!(
      "removed stale Chromium data from {}: {} ({} bytes freed)",
      root.display(),
      tidied.removed.join(", "),
      tidied.freed
    );
  }
  tidied
}

/// The name of the directory directly under `root` that holds `path`, if it is under `root`.
fn top_level_name(root: &Path, path: &Path) -> Option<String> {
  match path.strip_prefix(root).ok()?.components().next()? {
    std::path::Component::Normal(name) => name.to_str().map(str::to_string),
    _ => None,
  }
}

/// The bytes in the files under `path`, not following symbolic links. Only what is removed is
/// measured.
fn size_of(path: &Path) -> u64 {
  let Ok(meta) = fs::symlink_metadata(path) else {
    return 0;
  };
  if !meta.is_dir() {
    return meta.len();
  }
  fs::read_dir(path).map_or(0, |entries| {
    entries.flatten().map(|entry| size_of(&entry.path())).sum()
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
      "tauri-cef-stale-data-{name}-{}",
      std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
  }

  fn file(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; bytes]).unwrap();
  }

  fn names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
      .unwrap()
      .map(|e| e.unwrap().file_name().into_string().unwrap())
      .collect();
    names.sort();
    names
  }

  /// A root cache path as Chromium leaves it, with nothing stale: the current profile, `Default`
  /// and the files and directories Chromium uses.
  fn clean_root(name: &str) -> (PathBuf, PathBuf) {
    let root = temp_root(name);
    let current = root.join("Profile-current");
    file(&current.join("Local Storage/leveldb/000003.log"), 10);
    file(&current.join("Preferences"), 10);
    file(&root.join("Default/Preferences"), 7);
    for dir in [
      "Dictionaries",
      "GPUPersistentCache",
      "Crowd Deny",
      "FileTypePolicies",
      "PKIMetadata",
      "Safe Browsing",
      "SafetyTips",
      "Subresource Filter",
      "TrustTokenKeyCommitments",
      "segmentation_platform",
      "DataStore-0000",
    ] {
      file(&root.join(dir).join("data"), 1);
    }
    for f in ["Local State", "Last Version", "first_party_sets.db"] {
      file(&root.join(f), 2);
    }
    file(&root.join("cef.log"), 900);
    file(&root.join("Profile-file"), 4);
    (root, current)
  }

  /// An upgraded root: an old profile, components and an oversized log go; the current
  /// profile, `Default` and what Chromium uses stay. A later start with the same stale names
  /// back removes them again.
  #[test]
  fn stale_data_goes_at_every_start() {
    let (root, current) = clean_root("upgraded");
    let before = names(&root);
    file(&root.join("Profile-old/Local Storage/leveldb/000003.log"), 100);
    file(&root.join("Profile-old/Preferences"), 5);
    file(&root.join("WidevineCdm/4.10/manifest.json"), 20);
    file(&root.join("hyphen-data/120/hyph-en-us.hyb"), 3);
    file(&root.join("chrome_debug.log"), LOG_LIMIT as usize + 1);

    let tidied = tidy(&root, Some(&current));
    assert_eq!(
      tidied,
      Tidied {
        removed: vec![
          "Profile-old".into(),
          "WidevineCdm".into(),
          "chrome_debug.log".into(),
          "hyphen-data".into()
        ],
        freed: 128 + LOG_LIMIT + 1,
      }
    );
    assert_eq!(names(&root), before);
    assert!(current.join("Local Storage/leveldb/000003.log").is_file());

    file(&root.join("Profile-older/Preferences"), 1);
    file(&root.join("WasmTtsEngine/1/x"), 1);
    assert_eq!(
      tidy(&root, Some(&current)).removed,
      ["Profile-older", "WasmTtsEngine"]
    );
    assert_eq!(names(&root), before);
    fs::remove_dir_all(&root).unwrap();
  }

  /// A tidy root loses nothing, and a missing one is no error.
  #[test]
  fn a_clean_or_missing_root_is_left_alone() {
    let (root, current) = clean_root("clean");
    let before = names(&root);
    assert_eq!(tidy(&root, Some(&current)), Tidied::default());
    assert_eq!(names(&root), before);
    assert_eq!(
      tidy(&root.join("missing"), Some(&current)),
      Tidied::default()
    );
    fs::remove_dir_all(&root).unwrap();
  }

  /// The check at every start stays cheap: a tidy root, as Chromium leaves it, takes well under
  /// a millisecond (the best of a few runs, so a busy machine doesn't fail it).
  #[test]
  fn a_clean_root_takes_under_a_millisecond() {
    let (root, current) = clean_root("timing");
    let best = (0..20)
      .map(|_| {
        let start = std::time::Instant::now();
        tidy(&root, Some(&current));
        start.elapsed()
      })
      .min()
      .unwrap();
    assert!(
      best < std::time::Duration::from_millis(1),
      "{best:?}"
    );
    fs::remove_dir_all(&root).unwrap();
  }

  /// Webviews on `Default` (no data directory, as on macOS) make every `Profile-*` stale.
  #[test]
  fn on_the_default_profile_every_derived_profile_is_stale() {
    let root = temp_root("default");
    file(&root.join("Default/Preferences"), 1);
    file(&root.join("Profile-a/Preferences"), 1);
    assert_eq!(tidy(&root, None).removed, ["Profile-a"]);
    assert_eq!(names(&root), ["Default"]);
    fs::remove_dir_all(&root).unwrap();
  }

  /// Until the current profile exists, other profiles stay; the components go anyway.
  #[test]
  fn other_profiles_wait_for_the_current_one() {
    let root = temp_root("deferred");
    let current = root.join("Profile-current");
    file(&root.join("Profile-old/Preferences"), 1);
    file(&root.join("ZxcvbnData/3/passwords.txt"), 2);
    assert_eq!(tidy(&root, Some(&current)).removed, ["ZxcvbnData"]);
    assert_eq!(names(&root), ["Profile-old"]);

    fs::create_dir_all(&current).unwrap();
    assert_eq!(tidy(&root, Some(&current)).removed, ["Profile-old"]);
    // A current profile outside the root can't be proven current: other profiles stay.
    file(&root.join("Profile-old/Preferences"), 1);
    assert_eq!(
      tidy(&root, Some(Path::new("/elsewhere/Profile-old"))),
      Tidied::default()
    );
    fs::remove_dir_all(&root).unwrap();
  }

  /// A profile nested deeper under the root keeps its top-level directory.
  #[test]
  fn the_current_profile_is_named_by_its_top_level_directory() {
    let root = Path::new("/cache/cef");
    assert_eq!(
      top_level_name(root, Path::new("/cache/cef/Profile-x/inner")).as_deref(),
      Some("Profile-x")
    );
    assert_eq!(top_level_name(root, Path::new("/elsewhere/Profile-x")), None);
    assert_eq!(top_level_name(root, root), None);
  }

  /// A symbolic link under a stale name is no directory of Chromium's: it and what it points to
  /// stay.
  #[cfg(unix)]
  #[test]
  fn a_linked_component_is_left_alone() {
    let root = temp_root("link");
    let outside = temp_root("link-target");
    file(&outside.join("keep.txt"), 3);
    std::os::unix::fs::symlink(&outside, root.join("WasmTtsEngine")).unwrap();
    std::os::unix::fs::symlink(outside.join("keep.txt"), root.join("cef.log")).unwrap();
    assert_eq!(tidy(&root, None), Tidied::default());
    assert!(root.join("WasmTtsEngine").is_symlink());
    assert!(outside.join("keep.txt").is_file());
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&outside).unwrap();
  }
}
