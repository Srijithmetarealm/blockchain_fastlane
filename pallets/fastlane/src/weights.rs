// ============================================================
// pallets/fastlane/src/weights.rs
// Weight definitions for the FastLane pallet.
//
// The values below are PLACEHOLDERS — sized as conservative
// upper bounds so the chain is not under-charging for these
// extrinsics. Replace with `cargo run --release --features
// runtime-benchmarks ... benchmark pallet` output before mainnet.
// ============================================================

#![allow(unused_parens)]

use frame_support::{traits::Get, weights::Weight};

pub trait WeightInfo {
    fn submit() -> Weight;
    fn attest() -> Weight;
    fn attest_unsigned() -> Weight;
    fn finalize() -> Weight;
    fn commit_inclusion() -> Weight;
    fn set_authorities(a: u32) -> Weight;
    fn set_threshold() -> Weight;
    fn post_bond() -> Weight;
    fn slash() -> Weight;
    fn on_initialize_expire(p: u32) -> Weight;
    fn on_initialize_finalize(p: u32) -> Weight;
    // §14 BLS. A BLS12-381 pairing verify runs IN the runtime (no host
    // function) and is far heavier than an sr25519 verify — these must be
    // weighted accordingly or they under-price and become a DoS vector.
    fn register_bls_key() -> Weight;
    fn attest_bls() -> Weight;
    fn register_bls_key_unsigned() -> Weight;
}

/// Conservative placeholder weights. Read/write counts are accurate; the
/// per-call ref_time is intentionally generous (~50ms) so a non-benchmarked
/// chain does not under-price these extrinsics.
const READ: u64 = 25_000_000;
const WRITE: u64 = 100_000_000;
const VERIFY_SIG: u64 = 60_000_000;
// One in-runtime BLS12-381 pairing verify. Conservative upper bound (~6ms) —
// ~100x an sr25519 verify — until formal benchmarks replace it. Deliberately
// generous so BLS calls cannot be under-priced (the DoS concern).
const BLS_VERIFY: u64 = 6_000_000_000;

pub struct SubstrateWeight<T>(core::marker::PhantomData<T>);

impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
    fn submit() -> Weight {
        Weight::from_parts(50_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(4))
            .saturating_add(T::DbWeight::get().writes(4))
    }

    fn attest() -> Weight {
        Weight::from_parts(50_000_000 + VERIFY_SIG, 0)
            .saturating_add(T::DbWeight::get().reads(4))
            .saturating_add(T::DbWeight::get().writes(2))
    }

    fn attest_unsigned() -> Weight {
        Self::attest()
    }

    fn finalize() -> Weight {
        Weight::from_parts(30_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(2))
            .saturating_add(T::DbWeight::get().writes(2))
    }

    fn commit_inclusion() -> Weight {
        // One sig verification plus a bounded read/write of the commitment list.
        Weight::from_parts(30_000_000 + VERIFY_SIG, 0)
            .saturating_add(T::DbWeight::get().reads(3))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    fn set_authorities(a: u32) -> Weight {
        Weight::from_parts(20_000_000, 0)
            .saturating_add(Weight::from_parts(READ.saturating_mul(a as u64), 0))
            .saturating_add(T::DbWeight::get().reads(a as u64 + 1))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    fn set_threshold() -> Weight {
        Weight::from_parts(15_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(1))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    fn post_bond() -> Weight {
        Weight::from_parts(40_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(2))
            .saturating_add(T::DbWeight::get().writes(2))
    }

    fn slash() -> Weight {
        Weight::from_parts(50_000_000 + VERIFY_SIG, 0)
            .saturating_add(T::DbWeight::get().reads(4))
            .saturating_add(T::DbWeight::get().writes(3))
    }

    fn on_initialize_expire(p: u32) -> Weight {
        Weight::from_parts(10_000_000, 0)
            .saturating_add(Weight::from_parts(READ.saturating_mul(p as u64), 0))
            .saturating_add(T::DbWeight::get().reads(p as u64 + 1))
            .saturating_add(T::DbWeight::get().writes(p as u64))
    }

    fn on_initialize_finalize(p: u32) -> Weight {
        Weight::from_parts(10_000_000, 0)
            .saturating_add(Weight::from_parts(WRITE.saturating_mul(p as u64), 0))
            .saturating_add(T::DbWeight::get().reads(p as u64 + 1))
            .saturating_add(T::DbWeight::get().writes(p as u64))
    }

    fn register_bls_key() -> Weight {
        // Validate the pubkey (one BLS deserialisation) + write.
        Weight::from_parts(BLS_VERIFY / 4, 0)
            .saturating_add(T::DbWeight::get().reads(2))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    fn attest_bls() -> Weight {
        // One BLS verify of the authority's signature, plus at threshold an
        // aggregation over the collected signatures.
        Weight::from_parts(BLS_VERIFY.saturating_add(50_000_000), 0)
            .saturating_add(T::DbWeight::get().reads(5))
            .saturating_add(T::DbWeight::get().writes(2))
    }

    fn register_bls_key_unsigned() -> Weight {
        // sr25519 proof verify + BLS pubkey validation + write.
        Weight::from_parts(VERIFY_SIG.saturating_add(BLS_VERIFY / 4), 0)
            .saturating_add(T::DbWeight::get().reads(2))
            .saturating_add(T::DbWeight::get().writes(1))
    }
}

impl WeightInfo for () {
    fn submit() -> Weight { Weight::from_parts(50_000_000, 0) }
    fn attest() -> Weight { Weight::from_parts(110_000_000, 0) }
    fn attest_unsigned() -> Weight { Weight::from_parts(110_000_000, 0) }
    fn finalize() -> Weight { Weight::from_parts(30_000_000, 0) }
    fn commit_inclusion() -> Weight { Weight::from_parts(90_000_000, 0) }
    fn set_authorities(_: u32) -> Weight { Weight::from_parts(50_000_000, 0) }
    fn set_threshold() -> Weight { Weight::from_parts(15_000_000, 0) }
    fn post_bond() -> Weight { Weight::from_parts(40_000_000, 0) }
    fn slash() -> Weight { Weight::from_parts(110_000_000, 0) }
    fn on_initialize_expire(_: u32) -> Weight { Weight::from_parts(10_000_000, 0) }
    fn on_initialize_finalize(_: u32) -> Weight { Weight::from_parts(10_000_000, 0) }
    fn register_bls_key() -> Weight { Weight::from_parts(1_500_000_000, 0) }
    fn attest_bls() -> Weight { Weight::from_parts(6_000_000_000, 0) }
    fn register_bls_key_unsigned() -> Weight { Weight::from_parts(1_560_000_000, 0) }
}
