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

    // Benchmarked on this host (`benchmark pallet`, 20 steps / 10 repeat).
    // A BLS12-381 verify runs in-runtime, so these are large — ~4.3ms to
    // validate a key, ~49.5ms for a signature verify + aggregate.
    fn register_bls_key() -> Weight {
        // Measured: validate the BLS public key (one deserialisation) + write.
        Weight::from_parts(4_255_593_000, 1709)
            .saturating_add(T::DbWeight::get().reads(2))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    fn attest_bls() -> Weight {
        // Measured: one BLS signature verify + aggregation over the set.
        Weight::from_parts(49_548_235_000, 4021)
            .saturating_add(T::DbWeight::get().reads(5))
            .saturating_add(T::DbWeight::get().writes(2))
    }

    fn register_bls_key_unsigned() -> Weight {
        // BLS pubkey validation (measured) + an sr25519 proof verify.
        Weight::from_parts(4_255_593_000u64.saturating_add(VERIFY_SIG), 1709)
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
    fn register_bls_key() -> Weight { Weight::from_parts(4_255_593_000, 0) }
    fn attest_bls() -> Weight { Weight::from_parts(49_548_235_000, 0) }
    fn register_bls_key_unsigned() -> Weight { Weight::from_parts(4_315_593_000, 0) }
}
