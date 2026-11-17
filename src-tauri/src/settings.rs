use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkerMode {
    Auto,
    Manual,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AutoScope {
    Volume,
    Folder,
}

fn automatic_worker_count(
    cpu_threads: usize,
    scope: AutoScope,
    conditions: crate::platform::SchedulingConditions,
) -> usize {
    let cpus = conditions.active_cpu_threads.unwrap_or(cpu_threads).max(1);
    let mut count = match scope {
        AutoScope::Volume => (cpus / 2).clamp(4, 8),
        AutoScope::Folder
            if cpus.min(cpu_threads) >= 12
                && conditions
                    .active_cpu_threads
                    .is_some_and(|active| active >= 12)
                && !conditions.low_power_mode
                && conditions.thermal_state == crate::platform::ThermalState::Nominal =>
        {
            // The additional worker is reserved for explicitly available large
            // CPUs. Pressure/unknown states retain the conservative policy.
            if cpus.min(cpu_threads) >= 14 {
                6
            } else {
                cpus.div_ceil(3).clamp(3, 5)
            }
        }
        AutoScope::Folder => (cpus / 4).clamp(3, 5),
    }
    .min(cpus)
    .min(cpu_threads.max(1))
    .min(disk_scan::MAX_WORKERS);
    if conditions.low_power_mode {
        count = (count / 2).max(1);
    }
    count = match conditions.thermal_state {
        crate::platform::ThermalState::Fair => count.saturating_sub(1).max(1),
        crate::platform::ThermalState::Serious | crate::platform::ThermalState::Critical => {
            (count / 2).max(1)
        }
        _ => count,
    };
    count
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInfo {
    pub workers: usize,
    pub worker_mode: WorkerMode,
    pub resolved_workers: usize,
    pub qos: disk_scan::ScanQos,
    pub qos_supported: bool,
    pub cpu_threads: usize,
    pub max_workers: usize,
    pub default_workers: usize,
    pub auto_scope: AutoScope,
    pub auto_base_workers: usize,
    pub low_power_mode: bool,
    pub thermal_state: crate::platform::ThermalState,
    pub warning: Option<String>,
    pub performance_threads: Option<usize>,
    pub efficiency_threads: Option<usize>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedSettings {
    workers: usize,
    #[serde(default)]
    qos: disk_scan::ScanQos,
    #[serde(default)]
    worker_mode: Option<WorkerMode>,
}
pub struct ScanSettings {
    info: SettingsInfo,
}
fn neutral_conditions() -> crate::platform::SchedulingConditions {
    crate::platform::SchedulingConditions {
        active_cpu_threads: None,
        low_power_mode: false,
        thermal_state: crate::platform::ThermalState::Unknown,
    }
}
impl Default for ScanSettings {
    fn default() -> Self {
        Self::with_cpu_threads(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(8),
        )
    }
}
impl ScanSettings {
    fn with_cpu_threads(cpu_threads: usize) -> Self {
        let cpu_threads = cpu_threads.max(1);
        let max_workers = cpu_threads.min(disk_scan::MAX_WORKERS);
        let default_workers =
            automatic_worker_count(cpu_threads, AutoScope::Volume, neutral_conditions());
        let (performance_threads, efficiency_threads) = crate::platform::cpu_thread_classes();
        Self {
            info: SettingsInfo {
                workers: default_workers,
                worker_mode: WorkerMode::Auto,
                resolved_workers: default_workers,
                qos: disk_scan::ScanQos::Automatic,
                qos_supported: cfg!(target_os = "macos"),
                cpu_threads,
                max_workers,
                default_workers,
                auto_scope: AutoScope::Volume,
                auto_base_workers: default_workers,
                low_power_mode: false,
                thermal_state: crate::platform::ThermalState::Unknown,
                warning: None,
                performance_threads,
                efficiency_threads,
            },
        }
    }
    pub fn load(config: &Path) -> Self {
        Self::load_for_cpu(
            config,
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(8),
        )
    }
    fn load_for_cpu(config: &Path, cpu_threads: usize) -> Self {
        let mut settings = Self::with_cpu_threads(cpu_threads);
        let record = config.join("scan-settings.json");
        let saved = (|| -> Result<SavedSettings, String> {
            let file = fs::File::open(&record).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(65537)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 65536 {
                return Err("Saved settings are too large.".into());
            }
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())
        })();
        match saved {
            Ok(saved) if (1..=disk_scan::MAX_WORKERS).contains(&saved.workers) => {
                settings.info.workers = saved.workers.min(settings.info.max_workers);
                settings.info.worker_mode = saved.worker_mode.unwrap_or(WorkerMode::Manual);
                if settings.info.qos_supported {
                    settings.info.qos = saved.qos;
                } else if saved.qos != disk_scan::ScanQos::Automatic {
                    settings.info.warning = Some("macOS scheduling preferences are unavailable on this platform. Using Automatic.".into());
                }
                if saved.workers > settings.info.max_workers {
                    settings.info.warning =
                        Some("The saved worker count was reduced to this Mac's CPU limit.".into());
                }
            }
            _ if !record.exists() => {}
            _ => {
                settings.info.warning = Some(
                    "Could not read valid saved settings. Using the recommended worker count."
                        .into(),
                )
            }
        }
        settings.info.resolved_workers = if settings.info.worker_mode == WorkerMode::Auto {
            settings.info.default_workers
        } else {
            settings.info.workers
        };
        settings
    }
    #[cfg(test)]
    pub fn info(&self) -> SettingsInfo {
        self.info_for_conditions(AutoScope::Volume, crate::platform::scheduling_conditions())
    }
    fn info_for_conditions(
        &self,
        scope: AutoScope,
        conditions: crate::platform::SchedulingConditions,
    ) -> SettingsInfo {
        let mut info = self.info.clone();
        info.auto_scope = scope;
        info.auto_base_workers = automatic_worker_count(
            info.cpu_threads,
            scope,
            crate::platform::SchedulingConditions {
                low_power_mode: false,
                thermal_state: crate::platform::ThermalState::Nominal,
                ..conditions
            },
        );
        info.default_workers = automatic_worker_count(info.cpu_threads, scope, conditions);
        info.low_power_mode = conditions.low_power_mode;
        info.thermal_state = conditions.thermal_state;
        info.resolved_workers = match info.worker_mode {
            WorkerMode::Manual => info.workers,
            WorkerMode::Auto => info.default_workers,
        };
        info
    }
    #[cfg(test)]
    pub fn resolve_workers(&self, scope: AutoScope) -> (usize, &'static str) {
        (
            self.automatic_info_for_scope(scope).resolved_workers,
            "hardware",
        )
    }
    /// Resolve both pools from one public scheduling snapshot; retired preferences stay ignored.
    pub fn resolve_scan_workers(
        &self,
        scope: AutoScope,
        conditions: crate::platform::SchedulingConditions,
    ) -> (usize, usize, &'static str) {
        let base = self
            .automatic_info_for_conditions(scope, conditions)
            .resolved_workers;
        let cap = if scope == AutoScope::Folder
            && !conditions.low_power_mode
            && conditions.thermal_state == crate::platform::ThermalState::Nominal
        {
            4.min(
                conditions
                    .active_cpu_threads
                    .unwrap_or(self.info.cpu_threads)
                    .max(1),
            )
            .min(self.info.cpu_threads.max(1))
            .min(disk_scan::MAX_WORKERS)
            .max(base)
        } else {
            base
        };
        (base, cap, "hardware")
    }
    /// Production scans use hardware Auto and inherited scheduling regardless of retired preferences.
    #[cfg(any(test, feature = "performance-benchmark"))]
    pub fn automatic_info_for_scope(&self, scope: AutoScope) -> SettingsInfo {
        self.automatic_info_for_conditions(scope, crate::platform::scheduling_conditions())
    }
    pub fn automatic_info_for_conditions(
        &self,
        scope: AutoScope,
        conditions: crate::platform::SchedulingConditions,
    ) -> SettingsInfo {
        let mut info = self.info_for_conditions(scope, conditions);
        info.worker_mode = WorkerMode::Auto;
        info.workers = info.default_workers;
        info.resolved_workers = info.default_workers;
        info.qos = disk_scan::ScanQos::Automatic;
        info.warning = None;
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caps_saved_workers_to_cpu_and_rejects_invalid_files_without_rewriting_them() {
        let config = tempfile::tempdir().unwrap();
        let record = config.path().join("scan-settings.json");
        fs::write(&record, r#"{"workers":12}"#).unwrap();
        assert_eq!(
            ScanSettings::load_for_cpu(config.path(), 32).info().workers,
            12
        );
        let small = ScanSettings::load_for_cpu(config.path(), 4);
        assert_eq!(small.info().workers, 4);
        assert!(small.info().warning.is_some());
        assert_eq!(ScanSettings::with_cpu_threads(0).info().max_workers, 1);
        for bytes in [
            b"bad json".to_vec(),
            b"{\"workers\":0}".to_vec(),
            vec![b' '; 70_000],
        ] {
            fs::write(&record, &bytes).unwrap();
            let loaded = ScanSettings::load_for_cpu(config.path(), 4);
            assert_eq!(loaded.info().workers, 4);
            assert!(loaded.info().warning.is_some());
            assert_eq!(fs::read(&record).unwrap(), bytes);
        }
    }
}

#[cfg(test)]
mod qos_tests {
    use super::*;
    #[test]
    fn legacy_settings_default_to_automatic_and_unknown_qos_is_rejected() {
        let config = tempfile::tempdir().unwrap();
        let record = config.path().join("scan-settings.json");
        fs::write(&record, r#"{"workers":3}"#).unwrap();
        let legacy = ScanSettings::load_for_cpu(config.path(), 14);
        assert_eq!(legacy.info().workers, 3);
        assert_eq!(legacy.info().qos, disk_scan::ScanQos::Automatic);
        fs::write(&record, r#"{"workers":3,"qos":"realtime"}"#).unwrap();
        let invalid = ScanSettings::load_for_cpu(config.path(), 14);
        assert_eq!(invalid.info().qos, disk_scan::ScanQos::Automatic);
        assert!(invalid.info().warning.is_some());
    }
}

#[cfg(test)]
mod auto_tests {
    use super::*;
    #[test]
    fn healthy_large_folder_scans_use_bounded_io_parallelism_only() {
        use crate::platform::{SchedulingConditions, ThermalState::*};
        for (cpu, expected) in [(8, 3), (10, 3), (12, 4), (13, 5), (14, 6), (16, 6), (32, 6)] {
            let healthy = SchedulingConditions {
                active_cpu_threads: Some(cpu),
                low_power_mode: false,
                thermal_state: Nominal,
            };
            assert_eq!(
                automatic_worker_count(cpu, AutoScope::Folder, healthy),
                expected
            );
        }
        for (power, thermal, expected) in [
            (false, Unknown, 3),
            (true, Nominal, 1),
            (false, Fair, 2),
            (false, Serious, 1),
            (false, Critical, 1),
        ] {
            assert_eq!(
                automatic_worker_count(
                    14,
                    AutoScope::Folder,
                    SchedulingConditions {
                        active_cpu_threads: Some(14),
                        low_power_mode: power,
                        thermal_state: thermal,
                    }
                ),
                expected
            );
        }
        let settings = ScanSettings::with_cpu_threads(14);
        for (hardware, active) in [(14, 13), (13, 14)] {
            assert_eq!(
                automatic_worker_count(
                    hardware,
                    AutoScope::Folder,
                    SchedulingConditions {
                        active_cpu_threads: Some(active),
                        low_power_mode: false,
                        thermal_state: Nominal,
                    }
                ),
                5,
            );
        }
        let unavailable = settings.info_for_conditions(
            AutoScope::Folder,
            SchedulingConditions {
                active_cpu_threads: None,
                low_power_mode: false,
                thermal_state: Nominal,
            },
        );
        assert_eq!(unavailable.resolved_workers, 3);
        assert_eq!(unavailable.auto_base_workers, 3);
        let reduced = settings.info_for_conditions(
            AutoScope::Folder,
            SchedulingConditions {
                active_cpu_threads: Some(8),
                low_power_mode: false,
                thermal_state: Nominal,
            },
        );
        assert_eq!(reduced.resolved_workers, 3);
        assert_eq!(reduced.auto_base_workers, 3);
        let pressure = settings.info_for_conditions(
            AutoScope::Folder,
            SchedulingConditions {
                active_cpu_threads: Some(14),
                low_power_mode: true,
                thermal_state: Nominal,
            },
        );
        assert_eq!(pressure.auto_base_workers, 6);
        assert_eq!(pressure.resolved_workers, 1);
    }
    #[test]
    fn scope_counts_cpu_caps_and_power_thermal_reductions() {
        let neutral = neutral_conditions();
        for (cpu, volume, folder) in [
            (0, 1, 1),
            (1, 1, 1),
            (2, 2, 2),
            (4, 4, 3),
            (8, 4, 3),
            (10, 5, 3),
            (14, 7, 3),
            (16, 8, 4),
            (32, 8, 5),
        ] {
            assert_eq!(
                automatic_worker_count(cpu, AutoScope::Volume, neutral),
                volume
            );
            assert_eq!(
                automatic_worker_count(cpu, AutoScope::Folder, neutral),
                folder
            );
        }
        let condition = |low_power_mode, thermal_state| crate::platform::SchedulingConditions {
            active_cpu_threads: Some(14),
            low_power_mode,
            thermal_state,
        };
        use crate::platform::ThermalState::*;
        for (power, thermal, expected) in [
            (false, Nominal, 7),
            (false, Unknown, 7),
            (true, Nominal, 3),
            (false, Fair, 6),
            (false, Serious, 3),
            (false, Critical, 3),
            (true, Fair, 2),
            (true, Serious, 1),
        ] {
            assert_eq!(
                automatic_worker_count(14, AutoScope::Volume, condition(power, thermal)),
                expected
            );
        }
        assert_eq!(
            automatic_worker_count(14, AutoScope::Folder, condition(true, Critical)),
            1
        );
        assert_eq!(
            automatic_worker_count(
                14,
                AutoScope::Volume,
                crate::platform::SchedulingConditions {
                    active_cpu_threads: Some(2),
                    ..neutral
                }
            ),
            2
        );
        assert_eq!(
            automatic_worker_count(
                14,
                AutoScope::Volume,
                crate::platform::SchedulingConditions {
                    active_cpu_threads: Some(0),
                    ..neutral
                }
            ),
            1
        );
    }
    #[test]
    fn legacy_manual_preferences_still_load_but_never_resolve_to_new_scans() {
        let config = tempfile::tempdir().unwrap();
        let record = config.path().join("scan-settings.json");
        let conditions = neutral_conditions();
        let fresh = ScanSettings::load_for_cpu(config.path(), 14);
        assert_eq!(fresh.info().worker_mode, WorkerMode::Auto);
        assert_eq!(
            fresh
                .info_for_conditions(AutoScope::Volume, conditions)
                .resolved_workers,
            7
        );
        let old = r#"{"workers":2,"workerMode":"manual","calibrations":[{"volumeKey":"v","hardwareKey":"h","qos":"automatic","version":1,"workers":6,"calibratedAtUnixMs":1,"sampleDirectories":400,"sampleEntries":5000}]}"#;
        fs::write(&record, old).unwrap();
        let s = ScanSettings::load_for_cpu(config.path(), 14);
        assert_eq!(s.info().worker_mode, WorkerMode::Manual);
        assert_eq!(s.info().workers, 2);
        assert_eq!(
            s.automatic_info_for_conditions(AutoScope::Volume, conditions)
                .resolved_workers,
            7
        );
        fs::write(&record, r#"{"workers":3}"#).unwrap();
        let legacy = ScanSettings::load_for_cpu(config.path(), 14);
        assert_eq!(legacy.info().worker_mode, WorkerMode::Manual);
        assert_eq!(legacy.info().resolved_workers, 3);
    }
    #[test]
    fn production_auto_ignores_retired_manual_and_qos_without_rewriting_preferences() {
        let config = tempfile::tempdir().unwrap();
        let record = config.path().join("scan-settings.json");
        let old = r#"{"workers":1,"workerMode":"manual","qos":"background"}"#;
        fs::write(&record, old).unwrap();
        let s = ScanSettings::load_for_cpu(config.path(), 14);
        assert_eq!(s.info().worker_mode, WorkerMode::Manual);
        for scope in [AutoScope::Volume, AutoScope::Folder] {
            let info = s.automatic_info_for_scope(scope);
            assert_eq!(info.worker_mode, WorkerMode::Auto);
            assert_eq!(info.qos, disk_scan::ScanQos::Automatic);
            assert_eq!(info.resolved_workers, info.default_workers);
            assert_eq!(s.resolve_workers(scope).1, "hardware");
            let conditions = crate::platform::SchedulingConditions {
                active_cpu_threads: Some(14),
                low_power_mode: false,
                thermal_state: crate::platform::ThermalState::Nominal,
            };
            let (base, cap, _) = s.resolve_scan_workers(scope, conditions);
            assert_eq!(base, automatic_worker_count(14, scope, conditions));
            assert!(cap >= base);
        }
        assert_eq!(fs::read_to_string(record).unwrap(), old);
    }
    #[test]
    fn cached_validation_cap_uses_one_bounded_healthy_scheduling_snapshot() {
        use crate::platform::{SchedulingConditions, ThermalState};
        for total in [0, 1, 2, 3, 4, 14, 64] {
            let settings = ScanSettings::with_cpu_threads(total);
            for active in [None, Some(0), Some(1), Some(2), Some(4), Some(128)] {
                for thermal in [
                    ThermalState::Unknown,
                    ThermalState::Nominal,
                    ThermalState::Fair,
                    ThermalState::Serious,
                    ThermalState::Critical,
                ] {
                    for low_power_mode in [false, true] {
                        let conditions = SchedulingConditions {
                            active_cpu_threads: active,
                            thermal_state: thermal,
                            low_power_mode,
                        };
                        for scope in [AutoScope::Folder, AutoScope::Volume] {
                            let (base, cap, source) =
                                settings.resolve_scan_workers(scope, conditions);
                            let captured =
                                settings.automatic_info_for_conditions(scope, conditions);
                            assert_eq!(base, captured.resolved_workers);
                            assert_eq!(source, "hardware");
                            assert!(cap >= base && cap <= disk_scan::MAX_WORKERS);
                            assert!(cap <= total.max(1));
                            assert!(cap <= active.unwrap_or(total).max(1));
                            if low_power_mode
                                || thermal != ThermalState::Nominal
                                || scope == AutoScope::Volume
                            {
                                assert_eq!(cap, base);
                            } else {
                                assert_eq!(
                                    cap,
                                    base.max(
                                        4.min(total.max(1)).min(active.unwrap_or(total).max(1))
                                    )
                                );
                            }
                        }
                    }
                }
            }
        }
        let conditions = SchedulingConditions {
            active_cpu_threads: Some(14),
            low_power_mode: false,
            thermal_state: ThermalState::Nominal,
        };
        let settings = ScanSettings::with_cpu_threads(14);
        assert_eq!(
            settings.resolve_scan_workers(AutoScope::Folder, conditions),
            (6, 6, "hardware")
        );
        // A later public snapshot affects the next scan's resolution, not the captured pair.
        let captured = settings.resolve_scan_workers(AutoScope::Folder, conditions);
        assert_eq!(
            settings.resolve_scan_workers(
                AutoScope::Folder,
                SchedulingConditions {
                    low_power_mode: true,
                    ..conditions
                }
            ),
            (1, 1, "hardware")
        );
        assert_eq!(captured, (6, 6, "hardware"));
    }
}
