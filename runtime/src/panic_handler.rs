//! Panic handler for builds without `std`.

// TODO: Remove this file once `sp-io` is upgraded.
//       See https://github.com/paritytech/polkadot-bulletin-chain/issues/18.

use sp_core::LogLevelFilter;

// #[cfg(not(feature = "std"))]
// #[panic_handler]
// pub fn panic(info: &core::panic::PanicInfo) -> ! {
// 	let message = alloc::format!("{}", info);
// 	sp_io::logging::log(LogLevelFilter::Error, "runtime", message.as_bytes());
// 	unreachable!();
// }
