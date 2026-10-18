use crate::{mock::*, Error, Event, Something};
use frame_support::{assert_noop, assert_ok};

#[test]
fn it_works_for_default_value() {
	new_test_ext().execute_with(|| {
		// Go past genesis block so events get deposited
		System::set_block_number(1);
		// Dispatch a signed extrinsic.
		assert_ok!(Template::do_something(RuntimeOrigin::signed(1), 42));
		// Read pallet storage and assert an expected result.
		assert_eq!(Something::<Test>::get(), Some(42));
		// Assert that the correct event was deposited
		System::assert_last_event(Event::SomethingStored { something: 42, who: 1 }.into());
	});
}

#[test]
fn correct_error_for_none_value() {
	new_test_ext().execute_with(|| {
		// Ensure the expected error is thrown when no value is present.
		assert_noop!(Template::cause_error(RuntimeOrigin::signed(1)), Error::<Test>::NoneValue);
	});
}

#[test]
fn gated_call_succeeds_when_preconsensed() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		// APPROVED_ID passes the FastLane gate, so the state change applies.
		assert_ok!(Template::do_something_gated(RuntimeOrigin::signed(1), APPROVED_ID, 7));
		assert_eq!(Something::<Test>::get(), Some(7));
		System::assert_last_event(Event::SomethingStored { something: 7, who: 1 }.into());
	});
}

#[test]
fn gated_call_blocked_when_not_preconsensed() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		// REJECTED_ID fails the FastLane gate: the call errors and, crucially,
		// no state is mutated — proving FastLane is a mandatory checkpoint.
		assert!(Template::do_something_gated(RuntimeOrigin::signed(1), REJECTED_ID, 7).is_err());
		assert_eq!(Something::<Test>::get(), None);
	});
}
