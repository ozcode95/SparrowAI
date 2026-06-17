//! Hardware / OS probe. Cached to `system.json` so we don't pay the WMI cost
//! on every launch — re-probe is opt-in.

pub mod recommend;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sysinfo::System;

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuKind {
    Integrated,
    Discrete,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub vram_mb: Option<u64>,
    pub kind: GpuKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpuInfo {
    pub name: String,
    pub vendor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Specs {
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub cpu_brand: String,
    pub cpu_vendor: String,
    pub cpu_physical_cores: usize,
    pub cpu_logical_cores: usize,
    pub ram_total_mb: u64,
    pub ram_usable_mb: u64,
    pub gpus: Vec<GpuInfo>,
    pub npus: Vec<NpuInfo>,
    pub probed_at: String,
}

pub fn load_cached() -> Option<Specs> {
    let p = paths::system_cache().ok()?;
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn save_cached(specs: &Specs) -> Result<()> {
    let p = paths::system_cache()?;
    std::fs::write(p, serde_json::to_vec_pretty(specs)?)?;
    Ok(())
}

pub fn probe() -> Result<Specs> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpus = sys.cpus();
    let cpu_brand = cpus
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();
    let cpu_vendor = cpus
        .first()
        .map(|c| c.vendor_id().to_string())
        .unwrap_or_default();

    let logical_cores = cpus.len();
    let physical_cores = sys.physical_core_count().unwrap_or(logical_cores);

    let (gpus, npus) = platform_accelerators();

    Ok(Specs {
        os: System::name().unwrap_or_else(|| "unknown".into()),
        os_version: System::os_version().unwrap_or_else(|| "?".into()),
        arch: std::env::consts::ARCH.to_string(),
        cpu_brand,
        cpu_vendor,
        cpu_physical_cores: physical_cores,
        cpu_logical_cores: logical_cores,
        ram_total_mb: sys.total_memory() / 1024 / 1024,
        // `sys.available_memory()` is the *currently free* RAM — useless
        // for hardware assessment. Use total physical RAM so the
        // recommendation engine has a stable number to budget against.
        ram_usable_mb: sys.total_memory() / 1024 / 1024,
        gpus,
        npus,
        probed_at: Utc::now().to_rfc3339(),
    })
}

// ─── platform-specific GPU/NPU enumeration ──────────────────────────────────

#[cfg(target_os = "windows")]
fn platform_accelerators() -> (Vec<GpuInfo>, Vec<NpuInfo>) {
    win::enumerate().unwrap_or_default()
}

#[cfg(not(target_os = "windows"))]
fn platform_accelerators() -> (Vec<GpuInfo>, Vec<NpuInfo>) {
    // TODO: implement Linux (sysfs / lspci) + macOS (system_profiler) probes.
    (Vec::new(), Vec::new())
}

#[cfg(target_os = "windows")]
mod win {
    use super::*;
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename = "Win32_VideoController")]
    #[serde(rename_all = "PascalCase")]
    struct VideoController {
        name: Option<String>,
        adapter_compatibility: Option<String>,
    }

    pub fn enumerate() -> anyhow::Result<(Vec<GpuInfo>, Vec<NpuInfo>)> {
        let com = COMLibrary::new()?;
        let wmi = WMIConnection::new(com)?;
        let rows: Vec<VideoController> = wmi.query()?;

        // Fetch real VRAM via DXGI — WMI's `AdapterRAM` is a u32 that
        // overflows for cards >4 GB (e.g. RTX 4070 12 GB truncates to 0).
        let vram_map = dxgi_vram_map()?;

        let mut gpus = Vec::new();
        let mut npus = Vec::new();
        for r in rows {
            let name = r.name.unwrap_or_default();
            let vendor = r.adapter_compatibility.unwrap_or_default();
            let lname = name.to_lowercase();
            let vendor_l = vendor.to_lowercase();

            // Intel NPU surfaces as "Intel(R) AI Boost" on Meteor/Lunar Lake.
            if lname.contains("ai boost") || lname.contains("npu") {
                npus.push(NpuInfo { name, vendor });
                continue;
            }

            let kind = if vendor_l.contains("nvidia") || lname.contains("radeon rx") {
                GpuKind::Discrete
            } else if vendor_l.contains("intel") || lname.contains("uhd") || lname.contains("iris")
            {
                GpuKind::Integrated
            } else {
                GpuKind::Unknown
            };

            // Match WMI entry → DXGI adapter by name substring.
            // DXGI names are typically longer (e.g. "NVIDIA GeForce RTX 4070")
            // so we do a bidirectional contains check.
            let vram_mb = vram_map
                .iter()
                .find(|(dxgi_name, _)| {
                    name.len() > 3
                        && dxgi_name.len() > 3
                        && (dxgi_name.contains(&name) || name.contains(dxgi_name.as_str()))
                })
                .map(|(_, mb)| *mb);

            // Integrated GPUs don't have meaningful *dedicated* VRAM —
            // they share system RAM. DXGI reports ~0 for dedicated on
            // iGPUs, which is correct. Keep the field as None so the
            // recommendation engine falls back to system RAM budget.
            let vram_mb = match kind {
                GpuKind::Integrated => None,
                _ => vram_mb.filter(|&mb| mb > 64), // sanity floor: 64 MB
            };

            gpus.push(GpuInfo {
                name,
                vendor,
                vram_mb,
                kind,
            });
        }
        Ok((gpus, npus))
    }

    /// Enumerate GPUs via DXGI and return a `(adapter_name, dedicated_vram_mb)`
    /// map. DXGI reports correct 64‑bit VRAM; WMI's `AdapterRAM` u32 overflows
    /// for cards >4 GB.
    fn dxgi_vram_map() -> anyhow::Result<Vec<(String, u64)>> {
        use windows::Win32::Graphics::Dxgi::{
            CreateDXGIFactory1, IDXGIFactory1, DXGI_ERROR_NOT_FOUND,
        };

        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
            .map_err(|e| anyhow::anyhow!("CreateDXGIFactory1: {e}"))?;

        let mut out = Vec::new();
        for i in 0u32.. {
            let adapter = unsafe { factory.EnumAdapters1(i) };
            if let Err(ref e) = adapter {
                if e.code() == DXGI_ERROR_NOT_FOUND {
                    break;
                }
                // Skip adapters we can't query (e.g. software renderers).
                continue;
            }
            let adapter = adapter.unwrap();

            let desc = unsafe { adapter.GetDesc1() }.unwrap_or_default();
            let name = String::from_utf16_lossy(&desc.Description)
                .trim_end_matches('\0')
                .to_string();
            // `DedicatedVideoMemory` is a SIZE_T (usize on this target).
            // DXGI reports the real value even for >4 GB cards unlike
            // WMI's `AdapterRAM` u32.
            let vram_mb = desc.DedicatedVideoMemory as u64 / (1024 * 1024);

            out.push((name, vram_mb));
        }
        Ok(out)
    }
}
