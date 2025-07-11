//! Panic handler for builds without `std`.

use sp_core::LogLevel;

#[cfg(not(feature = "std"))]
#[panic_handler]
pub fn panic(info: &core::panic::PanicInfo) -> ! {
	let message = alloc::format!("{}", info);
	sp_io::logging::log(LogLevel::Error, "runtime", message.as_bytes());
	unreachable!();
}
