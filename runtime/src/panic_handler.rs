//! Panic handler for builds without `std`.

// TODO: Remove this file once `sp-io` is uipgraded.
//       See https://github.com/paritytech/polkadot-bulletin-chain/issues/18.

use sp_core::LogLevel;

#[cfg(not(feature = "std"))]
#[panic_handler]
pub fn panic(info: &core::panic::PanicInfo) -> ! {
	let message = alloc::format!("{}", info);
	sp_io::logging::log(LogLevel::Error, "runtime", message.as_bytes());
	unreachable!();
}
