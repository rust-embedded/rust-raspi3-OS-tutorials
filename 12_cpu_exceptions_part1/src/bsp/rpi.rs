// SPDX-License-Identifier: MIT
//
// Copyright (c) 2018-2019 Andre Richter <andre.o.richter@gmail.com>

//! Board Support Package for the Raspberry Pi.

mod memory_map;
mod virt_mem_layout;

use super::driver;
use crate::{interface, memory::KernelVirtualLayout};
use core::fmt;

pub const BOOT_CORE_ID: u64 = 0;
pub const BOOT_CORE_STACK_START: u64 = 0x80_000;

//--------------------------------------------------------------------------------------------------
// Global BSP driver instances
//--------------------------------------------------------------------------------------------------

static GPIO: driver::GPIO = unsafe { driver::GPIO::new(memory_map::mmio::GPIO_BASE) };
static PL011_UART: driver::PL011Uart =
    unsafe { driver::PL011Uart::new(memory_map::mmio::PL011_UART_BASE) };

//--------------------------------------------------------------------------------------------------
// Implementation of the kernel's BSP calls
//--------------------------------------------------------------------------------------------------

/// Board identification.
pub fn board_name() -> &'static str {
    #[cfg(feature = "bsp_rpi3")]
    {
        "Raspberry Pi 3"
    }

    #[cfg(feature = "bsp_rpi4")]
    {
        "Raspberry Pi 4"
    }
}

/// Return a reference to a `console::All` implementation.
pub fn console() -> &'static impl interface::console::All {
    &PL011_UART
}

/// In case of a panic, the panic handler uses this function to take a last shot at printing
/// something before the system is halted.
///
/// # Safety
///
/// - Use only for printing during a panic.
pub unsafe fn panic_console_out() -> impl fmt::Write {
    let uart = driver::PanicUart::new(memory_map::mmio::PL011_UART_BASE);
    uart.init();
    uart
}

/// Return an array of references to all `DeviceDriver` compatible `BSP` drivers.
///
/// # Safety
///
/// The order of devices is the order in which `DeviceDriver::init()` is called.
pub fn device_drivers() -> [&'static dyn interface::driver::DeviceDriver; 2] {
    [&GPIO, &PL011_UART]
}

/// BSP initialization code that runs after driver init.
pub fn post_driver_init() {
    // Configure PL011Uart's output pins.
    GPIO.map_pl011_uart();
}

/// Return the address space size in bytes.
pub const fn addr_space_size() -> usize {
    memory_map::END_INCLUSIVE + 1
}

/// Return a reference to the virtual memory layout.
pub fn virt_mem_layout() -> &'static KernelVirtualLayout<{ virt_mem_layout::NUM_MEM_RANGES }> {
    &virt_mem_layout::LAYOUT
}