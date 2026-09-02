//! The hand-verified static spec table (brief 2b, digest §4): what NVML cannot
//! say — SM/TMU/ROP/RT/tensor counts, L2, die, transistors, spec clocks, the
//! memory data rate and the board TDP — keyed by PCI device id. NVML's own
//! numbers (cores, bus width) are carried too, only to cross-check the row:
//! on disagreement NVML wins and `GpuInfo::spec_mismatch` is set.
//!
//! **`0x2B85` is the RTX 5090.** gpuwatch's database maps that id to a 5070
//! and files the 5090 under `2B06`; that file was never copied (digest §4).
//! The 5090 row was verified on torch against NVML (21760 cores, 512-bit) and
//! pci.ids; the other rows are NVIDIA's published specifications.

use std::borrow::Cow;

use gridwatch_store::keys::gpu::GpuSpec;

/// One table row: the identity NVML can confirm plus the GPU-Z column.
#[derive(Clone, Debug, PartialEq)]
pub struct SpecRow {
    pub pci_id: u32,
    pub name: &'static str,
    pub cores: u32,
    pub bus_width: u32,
    pub spec: GpuSpec,
}

macro_rules! row {
    ($id:expr, $name:expr, cores $cores:expr, bus $bus:expr, sms $sms:expr, tmus $tmus:expr,
     rops $rops:expr, rt $rt:expr, tensor $tensor:expr, l2 $l2:expr, base $base:expr,
     boost $boost:expr, gbps $gbps:expr, bw $bw:expr, tdp $tdp:expr, die $die:expr,
     tr $tr:expr, launch $launch:expr) => {
        SpecRow {
            pci_id: $id,
            name: $name,
            cores: $cores,
            bus_width: $bus,
            spec: GpuSpec {
                sms: $sms,
                tmus: $tmus,
                rops: $rops,
                rt_cores: $rt,
                tensor_cores: $tensor,
                l2_mb: $l2,
                base_mhz: $base,
                boost_mhz: $boost,
                mem_gbps: $gbps,
                bandwidth_gbs: $bw,
                tdp_w: $tdp,
                die_mm2: $die,
                transistors_b: $tr,
                launch: Cow::Borrowed($launch),
            },
        }
    };
}

pub static SPECS: &[SpecRow] = &[
    // Blackwell (GB20x)
    row!(0x2B85, "GeForce RTX 5090", cores 21760, bus 512, sms 170, tmus 680, rops 176, rt 170,
         tensor 680, l2 96, base 2017, boost 2407, gbps 28.0, bw 1792, tdp 575, die 750, tr 92.2,
         launch "2025-01-30"),
    row!(0x2B87, "GeForce RTX 5090 D", cores 21760, bus 512, sms 170, tmus 680, rops 176, rt 170,
         tensor 680, l2 96, base 2017, boost 2407, gbps 28.0, bw 1792, tdp 575, die 750, tr 92.2,
         launch "2025-01-30"),
    row!(0x2C02, "GeForce RTX 5080", cores 10752, bus 256, sms 84, tmus 336, rops 112, rt 84,
         tensor 336, l2 64, base 2295, boost 2617, gbps 30.0, bw 960, tdp 360, die 378, tr 45.6,
         launch "2025-01-30"),
    row!(0x2C05, "GeForce RTX 5070 Ti", cores 8960, bus 256, sms 70, tmus 280, rops 96, rt 70,
         tensor 280, l2 48, base 2295, boost 2452, gbps 28.0, bw 896, tdp 300, die 378, tr 45.6,
         launch "2025-02-20"),
    row!(0x2F04, "GeForce RTX 5070", cores 6144, bus 192, sms 48, tmus 192, rops 80, rt 48,
         tensor 192, l2 48, base 2160, boost 2512, gbps 28.0, bw 672, tdp 250, die 263, tr 31.1,
         launch "2025-03-05"),
    row!(0x2D04, "GeForce RTX 5060 Ti", cores 4608, bus 128, sms 36, tmus 144, rops 48, rt 36,
         tensor 144, l2 32, base 2407, boost 2572, gbps 28.0, bw 448, tdp 180, die 181, tr 21.9,
         launch "2025-04-16"),
    row!(0x2D05, "GeForce RTX 5060", cores 3840, bus 128, sms 30, tmus 120, rops 48, rt 30,
         tensor 120, l2 32, base 2280, boost 2497, gbps 28.0, bw 448, tdp 145, die 181, tr 21.9,
         launch "2025-05-19"),
    // Ada (AD10x)
    row!(0x2684, "GeForce RTX 4090", cores 16384, bus 384, sms 128, tmus 512, rops 176, rt 128,
         tensor 512, l2 72, base 2235, boost 2520, gbps 21.0, bw 1008, tdp 450, die 609, tr 76.3,
         launch "2022-10-12"),
    row!(0x2702, "GeForce RTX 4080 SUPER", cores 10240, bus 256, sms 80, tmus 320, rops 112, rt 80,
         tensor 320, l2 64, base 2295, boost 2550, gbps 23.0, bw 736, tdp 320, die 379, tr 45.9,
         launch "2024-01-31"),
    row!(0x2704, "GeForce RTX 4080", cores 9728, bus 256, sms 76, tmus 304, rops 112, rt 76,
         tensor 304, l2 64, base 2205, boost 2505, gbps 22.4, bw 717, tdp 320, die 379, tr 45.9,
         launch "2022-11-16"),
    row!(0x2705, "GeForce RTX 4070 Ti SUPER", cores 8448, bus 256, sms 66, tmus 264, rops 96, rt 66,
         tensor 264, l2 48, base 2340, boost 2610, gbps 21.0, bw 672, tdp 285, die 379, tr 45.9,
         launch "2024-01-24"),
];

pub fn lookup(pci_id: u32) -> Option<&'static SpecRow> {
    SPECS.iter().find(|r| r.pci_id == pci_id)
}

/// The row for a device plus whether NVML disagreed with it (cores or bus
/// width). `None` from NVML is not a disagreement — the field was
/// `NotSupported`, and the row still adds what it knows.
pub fn cross_check(
    pci_id: u32,
    cores: Option<u32>,
    bus_width: Option<u32>,
) -> (Option<GpuSpec>, bool) {
    let Some(row) = lookup(pci_id) else {
        return (None, false);
    };
    let mismatch =
        cores.is_some_and(|c| c != row.cores) || bus_width.is_some_and(|b| b != row.bus_width);
    (Some(row.spec.clone()), mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_5090_is_2b85_and_matches_nvml_on_torch() {
        let row = lookup(0x2B85).expect("5090 row");
        assert_eq!(row.name, "GeForce RTX 5090");
        assert_eq!((row.cores, row.bus_width), (21760, 512));
        assert_eq!(
            row.spec.l2_mb, 96,
            "the shipping 5090 has 96 MB, not GB202's 128"
        );
        assert_eq!(row.spec.die_mm2, 750);
        let (spec, mismatch) = cross_check(0x2B85, Some(21760), Some(512));
        assert!(spec.is_some() && !mismatch);
        // gpuwatch's wrong id must not resolve to anything.
        assert!(lookup(0x2B06).is_none());
    }

    #[test]
    fn ids_are_unique_and_rows_are_internally_consistent() {
        let mut ids: Vec<u32> = SPECS.iter().map(|r| r.pci_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SPECS.len(), "duplicate PCI id");
        for r in SPECS {
            // 128 CUDA cores per SM on Ada and Blackwell.
            assert_eq!(r.cores, r.spec.sms * 128, "{}", r.name);
            assert_eq!(r.spec.rt_cores, r.spec.sms, "{}", r.name);
            assert_eq!(r.spec.tmus, r.spec.sms * 4, "{}", r.name);
            // bandwidth = data rate × bus width / 8, within rounding.
            let bw = r.spec.mem_gbps as f64 * f64::from(r.bus_width) / 8.0;
            assert!(
                (bw - f64::from(r.spec.bandwidth_gbs)).abs() < 1.0,
                "{}: {bw} vs {}",
                r.name,
                r.spec.bandwidth_gbs
            );
            assert!(r.spec.boost_mhz > r.spec.base_mhz, "{}", r.name);
        }
    }

    #[test]
    fn nvml_wins_on_disagreement_and_absence_is_not_disagreement() {
        let (spec, mismatch) = cross_check(0x2B85, Some(20000), Some(512));
        assert!(spec.is_some() && mismatch);
        let (spec, mismatch) = cross_check(0x2B85, None, None);
        assert!(spec.is_some() && !mismatch);
        assert_eq!(cross_check(0xFFFF, Some(1), Some(1)), (None, false));
    }
}
