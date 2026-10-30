use super::Cartridge;

fn rom(cart_type: u8, size: u8, ram_size: u8) -> Vec<u8> {
    let banks = 2usize << size;
    let mut bytes = vec![0; banks * 0x4000];
    for bank in 0..banks {
        bytes[bank * 0x4000] = bank as u8;
        bytes[bank * 0x4000 + 0x3fff] = bank as u8;
    }
    bytes[0x147] = cart_type;
    bytes[0x148] = size;
    bytes[0x149] = ram_size;
    bytes
}

#[test]
fn mbc3_rom_banks_use_seven_bits_and_remap_zero() {
    let mut cart = Cartridge::new(&rom(0x13, 6, 3)).unwrap();
    assert_eq!(cart.read_rom(0x4000), 1);
    for register in [0x2000, 0x2fff, 0x3000, 0x3fff] {
        for value in 0..=u8::MAX {
            cart.write_rom(register, value);
            let expected = (value & 0x7f).max(1);
            assert_eq!(cart.read_rom(0x4000), expected);
            assert_eq!(cart.read_rom(0x7fff), expected);
            assert_eq!(cart.read_rom(0), 0);
            assert_eq!(cart.read_rom(0x3fff), 0);
        }
    }
    // MBC3 can select banks 0x20/0x40/0x60 without MBC1's special cases.
    // On a smaller chip, disconnected ROM address bits are masked.
    let mut smaller = Cartridge::new(&rom(0x13, 5, 3)).unwrap();
    smaller.write_rom(0x2000, 0x7f);
    assert_eq!(smaller.read_rom(0x4000), 0x3f);
    smaller.write_rom(0x2000, 0x40);
    assert_eq!(smaller.read_rom(0x4000), 0);
}

#[test]
fn mbc3_ram_banks_are_independent_and_enable_gated() {
    for cart_type in [0x12, 0x13] {
        let mut cart = Cartridge::new(&rom(cart_type, 5, 3)).unwrap();
        assert_eq!(cart.ram().len(), 0x8000);
        cart.write_ram(0xa000, 0xff);
        assert_eq!(cart.read_ram(0xa000), 0xff);
        cart.write_rom(0x1fff, 0x1a); // Only the low nibble enables RAM.
        assert_eq!(cart.read_ram(0xa000), 0); // Disabled write did not stick.
        for bank in 0..4 {
            cart.write_rom(0x4000, bank);
            cart.write_ram(0xa000, 0x10 + bank);
            cart.write_ram(0xbfff, 0x20 + bank);
        }
        // Clock-latch writes on a non-clock cartridge do not change RAM banks.
        cart.write_rom(0x6000, 0);
        cart.write_rom(0x7fff, 1);
        assert_eq!(cart.read_ram(0xa000), 0x13);
        for bank in 0..4 {
            cart.write_rom(0x5fff, bank);
            assert_eq!(cart.read_ram(0xa000), 0x10 + bank);
            assert_eq!(cart.read_ram(0xbfff), 0x20 + bank);
            cart.write_rom(0, 0);
            assert_eq!(cart.read_ram(0xa000), 0xff);
            cart.write_ram(0xa000, 0xee);
            cart.write_rom(0, 0x0a);
            assert_eq!(cart.read_ram(0xa000), 0x10 + bank);
        }
    }
}

#[test]
fn mbc3_absent_rtc_and_invalid_selectors_do_not_alias_ram() {
    let mut cart = Cartridge::new(&rom(0x13, 5, 3)).unwrap();
    cart.write_rom(0, 0x0a);
    for bank in 0..4 {
        cart.write_rom(0x4000, bank);
        cart.write_ram(0xa000, bank + 1);
    }
    for selector in 4..=u8::MAX {
        cart.write_rom(0x4000, selector);
        assert_eq!(cart.read_ram(0xa000), 0xff);
        cart.write_ram(0xa000, 0xee);
    }
    for bank in 0..4 {
        cart.write_rom(0x4000, bank);
        assert_eq!(cart.read_ram(0xa000), bank + 1);
    }
}

#[test]
fn mbc3_cartridge_types_and_capacity_are_explicit() {
    let mut bare = Cartridge::new(&rom(0x11, 0, 0)).unwrap();
    bare.write_rom(0, 0x0a);
    bare.write_ram(0xa000, 0x42);
    assert_eq!(bare.read_ram(0xa000), 0xff);
    assert!(bare.ram().is_empty());
    // A RAM-less header must not allocate phantom RAM.
    assert!(Cartridge::new(&rom(0x13, 0, 0)).unwrap().ram().is_empty());
    assert_eq!(
        Cartridge::new(&rom(0x12, 0, 2)).unwrap().ram().len(),
        0x2000
    );
    for rtc_type in [0x0f, 0x10] {
        assert!(Cartridge::new(&rom(rtc_type, 0, 3)).is_none());
    }
    assert!(Cartridge::new(&rom(0x13, 7, 3)).is_none()); // Beyond MBC3's 2 MiB.
    assert!(Cartridge::new(&rom(0x13, 5, 5)).is_none()); // Beyond 32 KiB RAM.
    let mut truncated = rom(0x13, 5, 3);
    truncated.pop();
    assert!(Cartridge::new(&truncated).is_none());
}
