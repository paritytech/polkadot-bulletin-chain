//! Slot-specific behavioural coverage for the per-scope [`crate::Authorization`]
//! redesign. Window arithmetic uses the **mock relay** block, advanced via
//! [`crate::mock::set_relay_now`].

use super::*;
use crate::{mock::set_relay_now, TimedAuthorization};

type Authorizations = super::Authorizations;

/// Convenience: read the slots vec for `scope`.
fn slots_for(scope: AuthorizationScope<u64>) -> Vec<TimedAuthorization> {
	Authorizations::get(scope).map(|a| a.slots.into_inner()).unwrap_or_default()
}

/// Slots are stored sorted by `expiration` ASC (tiebreak `starts_at` ASC),
/// regardless of the order in which they were authorized.
#[test]
fn slot_ordering_invariant_holds_after_unsorted_inserts() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		// Window pushes are intentionally out of order: 200, 150, 175.
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
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			100,
			Some(100),
			175,
		));
		let slots = slots_for(AuthorizationScope::Account(who));
		assert_eq!(slots.len(), 3);
		assert_eq!(slots[0].expiration, 150);
		assert_eq!(slots[1].expiration, 175);
		assert_eq!(slots[2].expiration, 200);
	});
}

/// Pushing a 9th distinct-window slot fails with `TooManySlots`. Pushing a
/// 9th matching exactly one of the existing windows is **additive** and
/// succeeds.
#[test]
fn max_slots_cap_rejects_distinct_but_accepts_additive() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		// Push 8 distinct-window slots.
		for i in 0..8u32 {
			assert_ok!(TransactionStorage::authorize_account_window(
				RuntimeOrigin::root(),
				who,
				1,
				100,
				Some(100),
				200 + i,
			));
		}
		// 9th distinct window — over the cap.
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
		// 9th matching the first slot's exact window — additive.
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			1,
			50,
			Some(100),
			200,
		));
		let slots = slots_for(AuthorizationScope::Account(who));
		assert_eq!(slots.len(), 8);
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
		let folded_before = TransactionStorage::account_authorization_extent(who);
		assert_eq!(folded_before.bytes, 100, "folded view clamps the over-cap");
		assert_eq!(folded_before.bytes_allowance, 100);
		assert_eq!(folded_before.transactions, 1);
		assert_eq!(folded_before.transactions_allowance, 1);

		// Re-authorize with the same window: the merge widens the caps but
		// must NOT expose the overage that the per-slot clamp was hiding.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 5, 500,));
		let folded_after = TransactionStorage::account_authorization_extent(who);
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

/// Consumption picks the earliest-expiring active slot. Store always
/// targets that slot regardless of byte counters (saturating). Renew
/// requires per-slot `bytes_permanent + size <= bytes_allowance`; when the
/// earliest doesn't fit, it falls through to the next active slot. No
/// cross-slot subsidy: a renew that doesn't fit any single slot rejects.
#[test]
fn consumption_picks_earliest_expiring_no_cross_slot_split() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		// Two slots: A (1500 bytes, earlier expiry), B (3000 bytes).
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			5,
			1500,
			Some(100),
			200,
		));
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			5,
			3000,
			Some(100),
			300,
		));

		// Store of 2000 bytes targets the earliest slot regardless of cap;
		// A's `bytes` saturates above its allowance.
		let call = Call::store { data: vec![0u8; 2000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		let slots = slots_for(AuthorizationScope::Account(who));
		let slot_a = slots.iter().find(|s| s.expiration == 200).unwrap();
		let slot_b = slots.iter().find(|s| s.expiration == 300).unwrap();
		assert_eq!(slot_a.extent.bytes, 2000, "store saturated slot A above its cap");
		assert_eq!(slot_b.extent.bytes, 0);

		// Pre-stage a 2000-byte payload to renew against.
		run_to_block(2, || None);
		BlockTransactions::kill();
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![7u8; 2000]));
		let block_txs = BlockTransactions::take();
		Transactions::insert(2u64, &block_txs);

		// Renew of 2000 bytes: slot A's `bytes_permanent + 2000 = 2000 > 1500`,
		// so falls through to slot B.
		let renew = Call::renew { block: 2, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew));
		let slots = slots_for(AuthorizationScope::Account(who));
		let slot_a = slots.iter().find(|s| s.expiration == 200).unwrap();
		let slot_b = slots.iter().find(|s| s.expiration == 300).unwrap();
		assert_eq!(slot_a.extent.bytes_permanent, 0, "renew did not borrow slot A");
		assert_eq!(slot_b.extent.bytes_permanent, 2000);
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
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent::default(),
		);
		// Advance relay to inside the window: now active on both signals.
		set_relay_now(150);
		assert!(TransactionStorage::account_has_active_authorization(&who));
		let extent = TransactionStorage::account_authorization_extent(who);
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
		let extent = TransactionStorage::account_authorization_extent(who);
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
		let extent = TransactionStorage::account_authorization_extent(who);
		assert_eq!(extent.bytes, 1000);
		assert_eq!(extent.transactions, 1);
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
	});
}

/// Expired slots are pruned. `remove_expired_account_authorization` then
/// finds the entry already gone and rejects with AuthorizationNotFound.
#[test]
fn expired_slots_pruned_on_read() {
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
		// Reading triggers the prune; the now-expired slot is removed.
		let extent = TransactionStorage::account_authorization_extent(who);
		assert_eq!(extent, AuthorizationExtent::default());
		assert!(!Authorizations::contains_key(AuthorizationScope::Account(who)));
	});
}

/// `bytes_permanent` is a per-slot hard cap. A renew never silently uses
/// a different slot's `bytes_permanent` headroom when this slot's is full.
#[test]
fn bytes_permanent_does_not_borrow_across_slots() {
	new_test_ext().execute_with(|| {
		set_relay_now(100);
		let who = 1u64;
		let data = vec![42u8; 2000];
		// Slot A: 2000-byte cap. Pre-fill `bytes_permanent` to its cap so
		// a 2000-byte renew cannot land here.
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			2,
			2000,
			Some(100),
			200,
		));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("auth exists");
			let slot_a = auth.slots.iter_mut().find(|s| s.expiration == 200).unwrap();
			slot_a.extent.bytes_permanent = 2000;
		});

		// Slot B (later expiration): big and unused.
		assert_ok!(TransactionStorage::authorize_account_window(
			RuntimeOrigin::root(),
			who,
			2,
			100_000,
			Some(100),
			300,
		));

		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(2, || None);

		// The renew must NOT pick slot A (bytes_permanent at cap). It picks
		// slot B and bumps that slot's `bytes_permanent`.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));
		let slots = slots_for(AuthorizationScope::Account(who));
		let slot_a = slots.iter().find(|s| s.expiration == 200).unwrap();
		let slot_b = slots.iter().find(|s| s.expiration == 300).unwrap();
		assert_eq!(slot_a.extent.bytes_permanent, 2000, "slot A's renew axis untouched");
		assert_eq!(slot_b.extent.bytes_permanent, 2000);
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
/// providers, the last slot expiring + a subsequent lazy prune dec's
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
		let _ = TransactionStorage::account_authorization_extent(who);
		assert_eq!(System::providers(&who), 1, "drained slot still holds the provider-ref");
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));

		// Advance past expiration; the next read prunes and dec's providers.
		set_relay_now(200);
		let _ = TransactionStorage::account_authorization_extent(who);
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
		for exp in [200u32, 150, 175] {
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
			TimedAuthorization,
			<Test as crate::Config>::MaxAuthorizationSlots,
		>::decode(&mut &raw[..])
		.expect("decode");
		let slots = decoded.into_inner();
		assert_eq!(slots.iter().map(|s| s.expiration).collect::<Vec<_>>(), vec![150, 175, 200]);
	});
}

/// Canonical accounting scenario: `bytes` (store) and `bytes_permanent`
/// (renew) are independent axes — both bounded per slot by
/// `bytes_allowance`. An account granted `N` bytes can store up to `N`
/// (saturating beyond) AND renew up to `N` (hard-capped).
#[test]
fn store_and_renew_axes_are_independent() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1u64;
		let n: u64 = 10_000;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 100, n));

		// store n bytes — bytes axis at cap; bytes_permanent untouched.
		let store_call = Call::store { data: vec![0u8; n as usize] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(2, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: n,
				bytes_permanent: 0,
				bytes_allowance: n,
				transactions: 1,
				transactions_allowance: 100,
			}
		);

		// renew n bytes — bytes_permanent axis at cap; bytes unchanged.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: n,
				bytes_permanent: n,
				bytes_allowance: n,
				transactions: 2,
				transactions_allowance: 100,
			}
		);

		// A second store of 1 byte still succeeds — store saturates and
		// the slot persists. The `transactions` counter is incremented
		// regardless of whether the store is high- or low-priority: it
		// feeds the boost decision in `AllowanceBasedPriority`, but
		// consumption itself never gates on it. The folded view clamps
		// `bytes` and `transactions` at their per-slot caps so a single
		// slot's overage can't mask another slot's headroom.
		let extra_store = Call::store { data: vec![1u8] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &extra_store));
		let extent_after_low_priority_store = TransactionStorage::account_authorization_extent(who);
		// Folded view: bytes clamped at bytes_allowance (= n).
		assert_eq!(extent_after_low_priority_store.bytes, n);
		// Tx allowance is 100, with 3 txs consumed → 3 (still under cap).
		assert_eq!(
			extent_after_low_priority_store.transactions, 3,
			"low-priority store still increments the transactions counter",
		);
		// Direct slot inspection still shows the raw saturated counters.
		let raw_slot = Authorizations::get(AuthorizationScope::Account(who))
			.expect("auth exists")
			.slots
			.into_inner()
			.into_iter()
			.next()
			.unwrap();
		assert_eq!(raw_slot.extent.bytes, n + 1);
		assert_eq!(raw_slot.extent.transactions, 3);

		// Another 1-byte renew is gated by the per-slot
		// `bytes_permanent + 1 > bytes_allowance` cap.
		//
		// Stage a 1-byte payload so the renew has a target.
		BlockTransactions::kill();
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![9u8; 1]));
		let block_txs = BlockTransactions::take();
		Transactions::insert(2u64, &block_txs);

		let small_renew = Call::renew { block: 2, index: 0 };
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&who, &small_renew),
			PERMANENT_ALLOWANCE_EXCEEDED,
		);
		// Slot still present (drained ≠ pruned); chain-wide counter unchanged.
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert_eq!(PermanentStorageUsed::get(), n);
	});
}
