# Tutorial 07 - UART Chainloader

## tl;dr

Running from an SD card was a nice experience, but it would be extremely tedious
to do it for every new binary. Let's write a [chainloader] using [position
independent code]. This will be the last binary you need to put on the SD card
for quite some time. Each following tutorial will provide a `chainboot` target in
the `Makefile` that lets you conveniently load the kernel over `UART`.

Our chainloader is called `MiniLoad` and is inspired by [raspbootin].

[chainloader]: https://en.wikipedia.org/wiki/Chain_loading
[position independent code]: https://en.wikipedia.org/wiki/Position-independent_code
[raspbootin]: https://github.com/mrvn/raspbootin

You can try it with this tutorial already:
1. Copy `kernel8.img` to the SD card.
2. Execute `make chainboot`.
3. Now plug in the USB Serial.
4. Let the magic happen.

In this tutorial, a version of the kernel from the previous tutorial is loaded
for demo purposes. In subsequent tuts, it will be the working directory's
kernel.

### Observing the jump

The `Makefile` in this tutorial has an additional target, `qemuasm`, that lets
you nicely observe the jump from the loaded address (`0x80_XXX`) to the
relocated code at (`0x3EFF_0XXX`):

```console
make qemuasm
[...]
IN:
0x000809fc:  d0000008  adrp     x8, #0x82000
0x00080a00:  52800020  movz     w0, #0x1
0x00080a04:  f9408908  ldr      x8, [x8, #0x110]
0x00080a08:  d63f0100  blr      x8

----------------
IN:
0x3eff0528:  d0000008  adrp     x8, #0x3eff2000
0x3eff052c:  d0000009  adrp     x9, #0x3eff2000
0x3eff0530:  f9411508  ldr      x8, [x8, #0x228]
0x3eff0534:  f9411929  ldr      x9, [x9, #0x230]
0x3eff0538:  eb08013f  cmp      x9, x8
0x3eff053c:  540000c2  b.hs     #0x3eff0554
[...]
```

## Diff to previous
```diff
Binary files 06_drivers_gpio_uart/demo_payload.img and 07_uart_chainloader/demo_payload.img differ

diff -uNr 06_drivers_gpio_uart/Makefile 07_uart_chainloader/Makefile
--- 06_drivers_gpio_uart/Makefile
+++ 07_uart_chainloader/Makefile
@@ -15,7 +15,7 @@
 	QEMU_MACHINE_TYPE = raspi3
 	QEMU_MISC_ARGS = -serial null -serial stdio
 	LINKER_FILE = src/bsp/rpi3/link.ld
-	RUSTC_MISC_ARGS = -C target-cpu=cortex-a53
+	RUSTC_MISC_ARGS = -C target-cpu=cortex-a53 -C relocation-model=pic
 endif

 SOURCES = $(wildcard **/*.rs) $(wildcard **/*.S) $(wildcard **/*.ld)
@@ -39,9 +39,14 @@

 DOCKER_CMD        = docker run -it --rm
 DOCKER_ARG_CURDIR = -v $(shell pwd):/work -w /work
-DOCKER_EXEC_QEMU  = $(QEMU_BINARY) -M $(QEMU_MACHINE_TYPE) -kernel $(OUTPUT)
+DOCKER_ARG_TTY    = --privileged -v /dev:/dev

-.PHONY: all doc qemu clippy clean readelf objdump nm
+DOCKER_EXEC_QEMU         = $(QEMU_BINARY) -M $(QEMU_MACHINE_TYPE) -kernel $(OUTPUT)
+DOCKER_EXEC_RASPBOOT     = raspbootcom
+DOCKER_EXEC_RASPBOOT_DEV = /dev/ttyUSB0
+# DOCKER_EXEC_RASPBOOT_DEV = /dev/ttyACM0
+
+.PHONY: all doc qemu qemuasm chainboot clippy clean readelf objdump nm

 all: clean $(OUTPUT)

@@ -60,6 +65,15 @@
 	$(DOCKER_CMD) $(DOCKER_ARG_CURDIR) $(CONTAINER_UTILS) \
 	$(DOCKER_EXEC_QEMU) $(QEMU_MISC_ARGS)

+qemuasm: all
+	$(DOCKER_CMD) $(DOCKER_ARG_CURDIR) $(CONTAINER_UTILS) \
+	$(DOCKER_EXEC_QEMU) -d in_asm
+
+chainboot:
+	$(DOCKER_CMD) $(DOCKER_ARG_CURDIR) $(DOCKER_ARG_TTY) \
+	$(CONTAINER_UTILS) $(DOCKER_EXEC_RASPBOOT) $(DOCKER_EXEC_RASPBOOT_DEV) \
+	demo_payload.img
+
 clippy:
 	cargo xclippy --target=$(TARGET) --features $(BSP)


diff -uNr 06_drivers_gpio_uart/src/arch/aarch64.rs 07_uart_chainloader/src/arch/aarch64.rs
--- 06_drivers_gpio_uart/src/arch/aarch64.rs
+++ 07_uart_chainloader/src/arch/aarch64.rs
@@ -23,7 +23,7 @@

     if bsp::BOOT_CORE_ID == MPIDR_EL1.get() & CORE_MASK {
         SP.set(bsp::BOOT_CORE_STACK_START);
-        crate::runtime_init::init()
+        crate::relocate::relocate_self::<u64>()
     } else {
         // if not core0, infinitely wait for events
         wait_forever()

diff -uNr 06_drivers_gpio_uart/src/bsp/driver/bcm/bcm2xxx_mini_uart.rs 07_uart_chainloader/src/bsp/driver/bcm/bcm2xxx_mini_uart.rs
--- 06_drivers_gpio_uart/src/bsp/driver/bcm/bcm2xxx_mini_uart.rs
+++ 07_uart_chainloader/src/bsp/driver/bcm/bcm2xxx_mini_uart.rs
@@ -251,6 +251,15 @@
         let mut r = &self.inner;
         r.lock(|inner| fmt::Write::write_fmt(inner, args))
     }
+
+    fn flush(&self) {
+        let mut r = &self.inner;
+        r.lock(|inner| loop {
+            if inner.AUX_MU_LSR.is_set(AUX_MU_LSR::TX_IDLE) {
+                break;
+            }
+        });
+    }
 }

 impl interface::console::Read for MiniUart {
@@ -267,14 +276,14 @@
             }

             // Read one character.
-            let mut ret = inner.AUX_MU_IO.get() as u8 as char;
-
-            // Convert carrige return to newline.
-            if ret == '
' {
-                ret = '
'
-            }
+            inner.AUX_MU_IO.get() as u8 as char
+        })
+    }

-            ret
+    fn clear(&self) {
+        let mut r = &self.inner;
+        r.lock(|inner| {
+            inner.AUX_MU_IIR.write(AUX_MU_IIR::FIFO_CLEAR::All);
         })
     }
 }

diff -uNr 06_drivers_gpio_uart/src/bsp/rpi3/link.ld 07_uart_chainloader/src/bsp/rpi3/link.ld
--- 06_drivers_gpio_uart/src/bsp/rpi3/link.ld
+++ 07_uart_chainloader/src/bsp/rpi3/link.ld
@@ -5,9 +5,10 @@

 SECTIONS
 {
-    /* Set current address to the value from which the RPi3 starts execution */
-    . = 0x80000;
+    /* Set the link address to the top-most 40 KiB of DRAM */
+    . = 0x3F000000 - 0x10000;

+    __binary_start = .;
     .text :
     {
         *(.text._start) *(.text*)
@@ -31,5 +32,14 @@
         __bss_end = .;
     }

+    .got :
+    {
+        *(.got*)
+    }
+
+    /* Fill up to 8 byte, b/c relocating the binary is done in u64 chunks */
+    . = ALIGN(8);
+    __binary_end = .;
+
     /DISCARD/ : { *(.comment*) }
 }

diff -uNr 06_drivers_gpio_uart/src/bsp/rpi3.rs 07_uart_chainloader/src/bsp/rpi3.rs
--- 06_drivers_gpio_uart/src/bsp/rpi3.rs
+++ 07_uart_chainloader/src/bsp/rpi3.rs
@@ -12,6 +12,9 @@
 pub const BOOT_CORE_ID: u64 = 0;
 pub const BOOT_CORE_STACK_START: u64 = 0x80_000;

+/// The address on which the RPi3 firmware loads every binary by default.
+pub const BOARD_DEFAULT_LOAD_ADDRESS: usize = 0x80_000;
+
 ////////////////////////////////////////////////////////////////////////////////
 // Global BSP driver instances
 ////////////////////////////////////////////////////////////////////////////////

diff -uNr 06_drivers_gpio_uart/src/interface.rs 07_uart_chainloader/src/interface.rs
--- 06_drivers_gpio_uart/src/interface.rs
+++ 07_uart_chainloader/src/interface.rs
@@ -26,6 +26,10 @@
     pub trait Write {
         fn write_char(&self, c: char);
         fn write_fmt(&self, args: fmt::Arguments) -> fmt::Result;
+
+        /// Block execution until the last character has been physically put on
+        /// the TX wire (draining TX buffers/FIFOs, if any).
+        fn flush(&self);
     }

     /// Console read functions.
@@ -33,6 +37,9 @@
         fn read_char(&self) -> char {
             ' '
         }
+
+        /// Clear RX buffers, if any.
+        fn clear(&self);
     }

     /// Console statistics.

diff -uNr 06_drivers_gpio_uart/src/main.rs 07_uart_chainloader/src/main.rs
--- 06_drivers_gpio_uart/src/main.rs
+++ 07_uart_chainloader/src/main.rs
@@ -23,8 +23,11 @@
 // `_start()` function, the first function to run.
 mod arch;

-// `_start()` then calls `runtime_init::init()`, which on completion, jumps to
-// `kernel_entry()`.
+// `_start()` then calls `relocate::relocate_self()`.
+mod relocate;
+
+// `relocate::relocate_self()` calls `runtime_init::init()`, which on
+// completion, jumps to `kernel_entry()`.
 mod runtime_init;

 // Conditionally includes the selected `BSP` code.
@@ -41,18 +44,48 @@
     // Run the BSP's initialization code.
     bsp::init();

-    println!("[0] Booting on: {}", bsp::board_name());
+    println!(" __  __ _      _ _                 _ ");
+    println!("|  \/  (_)_ _ (_) |   ___  __ _ __| |");
+    println!("| |\/| | | ' \| | |__/ _ \/ _` / _` |");
+    println!("|_|  |_|_|_||_|_|____\___/\__,_\__,_|");
+    println!();
+    println!("{:^37}", bsp::board_name());
+    println!();
+    println!("[ML] Requesting binary");
+    bsp::console().flush();
+
+    // Clear the RX FIFOs, if any, of spurious received characters before
+    // starting with the loader protocol.
+    bsp::console().clear();
+
+    // Notify raspbootcom to send the binary.
+    for _ in 0..3 {
+        bsp::console().write_char(3 as char);
+    }

-    println!("[1] Drivers loaded:");
-    for (i, driver) in bsp::device_drivers().iter().enumerate() {
-        println!("      {}. {}", i + 1, driver.compatible());
+    // Read the binary's size.
+    let mut size: u32 = u32::from(bsp::console().read_char() as u8);
+    size |= u32::from(bsp::console().read_char() as u8) << 8;
+    size |= u32::from(bsp::console().read_char() as u8) << 16;
+    size |= u32::from(bsp::console().read_char() as u8) << 24;
+
+    // Trust it's not too big.
+    print!("OK");
+
+    let kernel_addr: *mut u8 = bsp::BOARD_DEFAULT_LOAD_ADDRESS as *mut u8;
+    unsafe {
+        // Read the kernel byte by byte.
+        for i in 0..size {
+            *kernel_addr.offset(i as isize) = bsp::console().read_char() as u8;
+        }
     }

-    println!("[2] Chars written: {}", bsp::console().chars_written());
-    println!("[3] Echoing input now");
+    println!("[ML] Loaded! Executing the payload now
");
+    bsp::console().flush();

-    loop {
-        let c = bsp::console().read_char();
-        bsp::console().write_char(c);
-    }
+    // Use black magic to get a function pointer.
+    let kernel: extern "C" fn() -> ! = unsafe { core::mem::transmute(kernel_addr as *const ()) };
+
+    // Jump to loaded kernel!
+    kernel()
 }

diff -uNr 06_drivers_gpio_uart/src/relocate.rs 07_uart_chainloader/src/relocate.rs
--- 06_drivers_gpio_uart/src/relocate.rs
+++ 07_uart_chainloader/src/relocate.rs
@@ -0,0 +1,47 @@
+// SPDX-License-Identifier: MIT
+//
+// Copyright (c) 2018-2019 Andre Richter <andre.o.richter@gmail.com>
+
+//! Relocation code.
+
+/// Relocates the own binary from `bsp::BOARD_DEFAULT_LOAD_ADDRESS` to the
+/// `__binary_start` address from the linker script.
+///
+/// # Safety
+///
+/// - Only a single core must be active and running this function.
+/// - Function must not use the `bss` section.
+pub unsafe fn relocate_self<T>() -> ! {
+    extern "C" {
+        static __binary_start: usize;
+        static __binary_end: usize;
+    }
+
+    let binary_start_addr: usize = &__binary_start as *const _ as _;
+    let binary_end_addr: usize = &__binary_end as *const _ as _;
+    let binary_size_in_byte: usize = binary_end_addr - binary_start_addr;
+
+    // Get the relocation destination address from the linker symbol.
+    let mut reloc_dst_addr: *mut T = binary_start_addr as *mut T;
+
+    // The address of where the previous firmware loaded us.
+    let mut src_addr: *const T = crate::bsp::BOARD_DEFAULT_LOAD_ADDRESS as *const _;
+
+    // Copy the whole binary.
+    //
+    // This is essentially a `memcpy()` optimized for throughput by transferring
+    // in chunks of T.
+    let n = binary_size_in_byte / core::mem::size_of::<T>();
+    for _ in 0..n {
+        use core::ptr;
+
+        ptr::write_volatile::<T>(reloc_dst_addr, ptr::read_volatile::<T>(src_addr));
+        reloc_dst_addr = reloc_dst_addr.offset(1);
+        src_addr = src_addr.offset(1);
+    }
+
+    // Call `init()` through a trait object, causing the jump to use an absolute
+    // address to reach the relocated binary. An elaborate explanation can be
+    // found in the runtime_init.rs source comments.
+    crate::runtime_init::get().init()
+}

diff -uNr 06_drivers_gpio_uart/src/runtime_init.rs 07_uart_chainloader/src/runtime_init.rs
--- 06_drivers_gpio_uart/src/runtime_init.rs
+++ 07_uart_chainloader/src/runtime_init.rs
@@ -4,23 +4,44 @@

 //! Rust runtime initialization code.

-/// Equivalent to `crt0` or `c0` code in C/C++ world. Clears the `bss` section,
-/// then calls the kernel entry.
+/// We are outsmarting the compiler here by using a trait as a layer of
+/// indirection. Because we are generating PIC code, a static dispatch to
+/// `init()` would generate a relative jump from the callee to `init()`.
+/// However, when calling `init()`, code just finished copying the binary to the
+/// actual link-time address, and hence is still running at whatever location
+/// the previous loader has put it. So we do not want a relative jump, because
+/// it would not jump to the relocated code.
 ///
-/// Called from `BSP` code.
-///
-/// # Safety
-///
-/// - Only a single core must be active and running this function.
-pub unsafe fn init() -> ! {
-    extern "C" {
-        // Boundaries of the .bss section, provided by the linker script.
-        static mut __bss_start: u64;
-        static mut __bss_end: u64;
+/// By indirecting through a trait object, we can make use of the property that
+/// vtables store absolute addresses. So calling `init()` this way will kick
+/// execution to the relocated binary.
+pub trait RunTimeInit {
+    /// Equivalent to `crt0` or `c0` code in C/C++ world. Clears the `bss` section,
+    /// then calls the kernel entry.
+    ///
+    /// Called from `BSP` code.
+    ///
+    /// # Safety
+    ///
+    /// - Only a single core must be active and running this function.
+    unsafe fn init(&self) -> ! {
+        extern "C" {
+            // Boundaries of the .bss section, provided by the linker script.
+            static mut __bss_start: u64;
+            static mut __bss_end: u64;
+        }
+
+        // Zero out the .bss section.
+        r0::zero_bss(&mut __bss_start, &mut __bss_end);
+
+        crate::kernel_entry()
     }
+}

-    // Zero out the .bss section.
-    r0::zero_bss(&mut __bss_start, &mut __bss_end);
+struct Traitor;
+impl RunTimeInit for Traitor {}

-    crate::kernel_entry()
+/// Give the callee a `RunTimeInit` trait object.
+pub fn get() -> &'static dyn RunTimeInit {
+    &Traitor {}
 }
```