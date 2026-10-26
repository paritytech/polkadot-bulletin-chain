//! Slot-specific behavioural coverage for the per-scope [`crate::Authorization`]
//! redesign. Window arithmetic uses the **mock relay** block, advanced via
//! [`crate::mock::set_relay_now`].

use super::*;
use crate::{mock::set_relay_now, TimedAuthorizationFor};

type Authorizations = super::Authorizations;
type Slot = TimedAuthorizationFor<Test>;

/// Convenience: read the slots vec for `scope`.
fn slots_for(scope: AuthorizationScope<u64>) -> Vec<Slot> {
	Authorizations::get(scope).map(|a| a.slots.into_inner()).unwrap_or_default()
}

/// Usage folded over `who`'s active slots (per-slot counters clamped at their caps).
fn folded(who: u64) -> AuthorizationExtent<()> {
	TransactionStorage::get_active_authorization(&AuthorizationScope::Account(who))
		.map(|auth| auth.usage(TransactionStorage::relay_now()))
		.unwrap_or_default()
}

/// Slots are stored sorted by `expiration` ASC (tiebreak `starts_at` ASC),
/// regardless of the order in which they were authorized.
#[test]
fn slot_ordering_invariant_holds_after_unsorted_inserts() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		// Window pushes are intentionally out of order: 200, then 150.
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			100,
			Some(100),
			200,
		));
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			100,
			Some(100),
			150,
		));
		let slots = slots_for(AuthorizationScope::Account(who));
		assert_eq!(slots.len(), 2);
		assert_eq!(slots[0].expiration, 150);
		assert_eq!(slots[1].expiration, 200);
	});
}

/// Pushing a 3rd distinct-window slot fails with `TooManySlots` (the mock allows 2).
/// Pushing a 3rd matching exactly one of the existing windows is **additive** and
/// succeeds.
#[test]
fn max_slots_cap_rejects_distinct_but_accepts_additive() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		// Fill the per-scope bound with distinct windows.
		for i in 0..2u32 {
			assert_ok!(TransactionStorage::authorize_account_window(
				RuntimeOrigin::root(),
				who,
				1,
				100,
				Some(100),
				200 + i,
			));
		}
		// 3rd distinct window — over the cap.
		assert_noop!(
			TransactionStorage::authorize_account_window(
				RuntimeOrigin::root(),
				who,
				1,
				100,
				Some(100),
				999,
			),
			crate::Error::<Test>::TooManySlots,
		);
		// 3rd matching the first slot's exact window — additive.
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			50,
			Some(100),
			200,
		));
		let slots = slots_for(AuthorizationScope::Account(who));
		assert_eq!(slots.len(), 2);
		let first = slots.iter().find(|s| s.expiration == 200).unwrap();
		assert_eq!(first.extent.bytes_allowance, 150);
		assert_eq!(first.extent.transactions_allowance, 2);
	});
}

/// Merging an over-cap slot with a new push is a pure simplification: the
/// folded extent after the merge equals the folded extent that would have
/// been observed if the new slot were stored as a separate (still-empty)
/// entry. `bytes` and `transactions` are pre-clamped at the **old** caps
/// before the new caps are added.
#[test]
fn additive_merge_does_not_unhide_existing_overage() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1u64;
		// Initial slot: 100 bytes, 1 tx. Drive `bytes` over the cap with
		// a saturating low-priority store.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 100,));
		let store = Call::store { data: vec![0u8; 200] }; // 2× over byte cap
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store));
		let folded_before = folded(who);
		assert_eq!(folded_before.bytes, 100, "folded view clamps the over-cap");
		assert_eq!(folded_before.bytes_allowance, 100);
		assert_eq!(folded_before.transactions, 1);
		assert_eq!(folded_before.transactions_allowance, 1);

		// Re-authorize with the same window: the merge widens the caps but
		// must NOT expose the overage that the per-slot clamp was hiding.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 5, 500,));
		let folded_after = folded(who);
		// New caps: bytes 100 + 500 = 600, tx 1 + 5 = 6.
		assert_eq!(folded_after.bytes_allowance, 600);
		assert_eq!(folded_after.transactions_allowance, 6);
		// Used counters: pre-clamped at old caps before the merge → no
		// suddenly-visible overage. Equivalent to two separate slots
		// with `bytes = 100, 0` clamped to `(100, 500)`.
		assert_eq!(folded_after.bytes, 100);
		assert_eq!(folded_after.transactions, 1);
	});
}

/// Two already-active slots that share the same `expiration` are merged
/// even when their `starts_at` differ. A `starts_at` in the past is
/// observationally equivalent to `relay_now` for an already-active slot.
#[test]
fn additive_merge_folds_already_active_slots_with_same_expiration() {
	new_test_ext().execute_with(|| {
		let who = 1u64;
		// Push slot A at relay 100 with starts_at=100.
		set_relay_now(100);
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			1000,
			Some(100),
			300,
		));
		// Advance relay; push slot B with starts_at=relay_now=150 and the
		// same expiration. Both are already-active — they would have been
		// distinct under the old exact-match rule, but they fold now.
		set_relay_now(150);
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			2,
			500,
			Some(150),
			300,
		));
		let slots = slots_for(AuthorizationScope::Account(who));
		assert_eq!(slots.len(), 1, "second push folded into the first slot");
		assert_eq!(slots[0].starts_at, 100);
		assert_eq!(slots[0].expiration, 300);
		assert_eq!(slots[0].extent.bytes_allowance, 1500);
		assert_eq!(slots[0].extent.transactions_allowance, 3);

		// Future-only slots still require exact-match merge: same expiration
		// but `new.starts_at > relay_now` does NOT fold into an active slot.
		set_relay_now(160);
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			100,
			Some(200),
			300,
		));
		let slots = slots_for(AuthorizationScope::Account(who));
		assert_eq!(slots.len(), 2, "future-only slot stays distinct");
	});
}

/// `authorize_account_window` validation accepts a `starts_at` in the
/// past (slot is treated as already-active) but rejects: empty windows,
/// already-expired windows, and a `starts_at` more than
/// `MaxStartsAtFuture` blocks ahead.
#[test]
fn invalid_window_is_rejected() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		// starts_at < relay_now is accepted: an already-active slot.
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			100,
			Some(50),
			200,
		));
		// expiration == starts_at — empty window.
		assert_noop!(
			TransactionStorage::authorize_account_window(
				RuntimeOrigin::root(),
				who,
				1,
				100,
				Some(100),
				100,
			),
			crate::Error::<Test>::InvalidWindow,
		);
		// expiration <= relay_now — already-expired window.
		assert_noop!(
			TransactionStorage::authorize_account_window(
				RuntimeOrigin::root(),
				who,
				1,
				100,
				Some(50),
				100,
			),
			crate::Error::<Test>::InvalidWindow,
		);
		// MaxStartsAtFuture = 100 in the mock; `relay_now + 101` is over.
		assert_noop!(
			TransactionStorage::authorize_account_window(
				RuntimeOrigin::root(),
				who,
				1,
				100,
				Some(100 + 101),
				100 + 200,
			),
			crate::Error::<Test>::InvalidWindow,
		);
		// starts_at = None defaults to relay_now and accepts.
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			100,
			None,
			300,
		));
	});
}

/// A future-only slot owns a storage entry but does not yet count as an
/// active authorization. Both `account_has_active_authorization` and the
/// folded extent ignore it until `relay_now` reaches its `starts_at`.
#[test]
fn future_only_slot_is_inactive_until_starts_at() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			5,
			1000,
			Some(150),
			200,
		));
		// Slot exists in storage but is future-only: not active yet.
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(!TransactionStorage::account_has_active_authorization(&who));
		assert_eq!(folded(who), AuthorizationExtent::default(),);
		// Advance relay to inside the window: now active on both signals.
		set_relay_now(150);
		assert!(TransactionStorage::account_has_active_authorization(&who));
		let extent = folded(who);
		assert_eq!(extent.bytes_allowance, 1000);
		assert_eq!(extent.transactions_allowance, 5);
	});
}

/// Drained slots persist in storage until they expire — they can still
/// serve low-priority `store()` calls (which never gate on the byte or tx
/// caps). Only expiry triggers removal.
#[test]
fn drained_slot_persists() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			1000,
			Some(100),
			200,
		));
		let call = Call::store { data: vec![0u8; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		// Slot is drained on both bytes and tx but stays in storage.
		let extent = folded(who);
		assert_eq!(extent.bytes, 1000);
		assert_eq!(extent.bytes_allowance, 1000);
		assert_eq!(extent.transactions, 1);
		assert_eq!(extent.transactions_allowance, 1);
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert_eq!(System::providers(&who), 1);

		// A second over-cap store still succeeds (saturating); slot stays.
		// The folded view clamps `bytes` and `transactions` per slot at
		// their own caps, so the over-cap consumption is invisible at
		// this level (it surfaces in the priority boost path instead).
		let call = Call::store { data: vec![0u8; 1] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		let extent = folded(who);
		assert_eq!(extent.bytes, 1000);
		assert_eq!(extent.transactions, 1);
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
	});
}

/// Expired slots are pruned by the next mutating path (here: a store validation), which
/// also releases the entry once no slot remains.
#[test]
fn expired_slots_pruned_on_mutation() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			5,
			1000,
			Some(100),
			200,
		));
		set_relay_now(200);
		// Reads already see nothing active; storage still holds the expired slot.
		assert_eq!(folded(who), AuthorizationExtent::default());
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
		// The store validation prunes it and rejects.
		let call = Call::store { data: vec![0u8; 10] };
		assert_eq!(
			TransactionStorage::pre_dispatch_signed(&who, &call),
			Err(InvalidTransaction::Payment.into()),
		);
		assert!(!Authorizations::contains_key(AuthorizationScope::Account(who)));
	});
}

/// `RelayChainTimeUnavailable` (sentinel `0`) on `authorize_account` is
/// surfaced as `Error::RelayChainTimeUnavailable`.
#[test]
fn relay_time_sentinel_rejected() {
	new_test_ext().execute_with(|| {
		set_relay_now(0);
		assert_noop!(
			TransactionStorage::authorize_account(RuntimeOrigin::root(), 1, 1, 100),
			crate::Error::<Test>::RelayChainTimeUnavailable,
		);
	});
}

/// Provider-ref accounting: first slot push for an Account scope inc's
/// providers, the last slot expiring + the next lazy prune dec's
/// providers. Drained-but-still-active slots do **not** dec, since the
/// account is still authorized for low-priority stores.
#[test]
fn provider_ref_lifecycle() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		assert!(System::providers(&who).is_zero());
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			1000,
			Some(100),
			200,
		));
		assert_eq!(System::providers(&who), 1);
		// Drain the slot on both axes — provider-ref stays.
		let call = Call::store { data: vec![0u8; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		let _ = folded(who);
		assert_eq!(System::providers(&who), 1, "drained slot still holds the provider-ref");
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));

		// Advance past expiration; the next mutating path prunes and dec's providers.
		set_relay_now(200);
		let _ = TransactionStorage::pre_dispatch_signed(&who, &call);
		assert_eq!(System::providers(&who), 0);
		assert!(!Authorizations::contains_key(AuthorizationScope::Account(who)));
	});
}

/// SCALE round-trip: encoding a `BoundedVec<TimedAuthorization, _>` and
/// decoding it back yields the same value (deterministic, sorted).
#[test]
fn scale_roundtrip_is_stable() {
	use codec::Decode;
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		for exp in [200u32, 150] {
			assert_ok!(TransactionStorage::authorize_account_window(
				RuntimeOrigin::root(),
				who,
				1,
				100,
				Some(100),
				exp,
			));
		}
		let raw = Authorizations::get(AuthorizationScope::Account(who)).unwrap().encode();
		let decoded = polkadot_sdk_frame::deps::frame_support::BoundedVec::<
			Slot,
			<Test as crate::Config>::MaxAuthorizationSlots,
		>::decode(&mut &raw[..])
		.expect("decode");
		let slots = decoded.into_inner();
		assert_eq!(slots.iter().map(|s| s.expiration).collect::<Vec<_>>(), vec![150, 200]);
	});
}
