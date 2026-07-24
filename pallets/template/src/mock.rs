use crate as pallet_template;
use frame_support::derive_impl;
use sp_runtime::BuildStorage;

type Block = frame_system::mocking::MockBlock<Test>;

#[frame_support::runtime]
mod runtime {
	// The main runtime
	#[runtime::runtime]
	// Runtime Types to be generated
	#[runtime::derive(
		RuntimeCall,
		RuntimeEvent,
		RuntimeError,
		RuntimeOrigin,
		RuntimeFreezeReason,
		RuntimeHoldReason,
		RuntimeSlashReason,
		RuntimeLockId,
		RuntimeTask,
		RuntimeViewFunction
	)]
	pub struct Test;

	#[runtime::pallet_index(0)]
	pub type System = frame_system::Pallet<Test>;

	#[runtime::pallet_index(1)]
	pub type Template = pallet_template::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
}

/// A payload id the mock gate treats as pre-consensed (approved).
pub const APPROVED_ID: pallet_fastlane::PayloadId = [1u8; 32];
/// A payload id the mock gate treats as NOT pre-consensed (rejected).
pub const REJECTED_ID: pallet_fastlane::PayloadId = [9u8; 32];

/// Stand-in for FastLane's `EnsurePreConsensed` guard. Approves only
/// `APPROVED_ID` so tests can exercise both the pass and the block paths.
/// FastLane's real gate logic is covered by the fastlane pallet's own tests;
/// here we only prove the template pallet honours whatever the gate decides.
pub struct MockGate;
impl pallet_fastlane::EnsurePreConsensed for MockGate {
	fn ensure_preconsensed(
		payload_id: pallet_fastlane::PayloadId,
	) -> frame_support::pallet_prelude::DispatchResult {
		if payload_id == APPROVED_ID {
			Ok(())
		} else {
			Err(sp_runtime::DispatchError::Other("not pre-consensed"))
		}
	}
}

impl pallet_template::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = ();
	type PreConsensusGate = MockGate;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
	frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
