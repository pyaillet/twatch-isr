use esp_idf_hal::gpio::{InputPin, OutputPin};
use esp_idf_hal::i2c::{self, I2c, I2cError};

use esp_idf_sys::{
    self, gpio_int_type_t_GPIO_INTR_NEGEDGE, gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
    gpio_pullup_t_GPIO_PULLUP_DISABLE, EspError, GPIO_MODE_DEF_INPUT,
};

use embedded_hal_0_2::blocking::delay::DelayMs;
use embedded_hal_0_2::blocking::i2c::Write;
use embedded_hal_0_2::prelude::*;

use std::sync::atomic::{AtomicBool, Ordering};

use std::ops::{BitAnd, BitOr};

use bitmask_enum::bitmask;

static AXPXX_IRQ_TRIGGERED: AtomicBool = AtomicBool::new(false);

const GPIO_INTR: u8 = 35;

const AXP202_SLAVE_ADDR: u8 = 0x35;

#[derive(Debug)]
pub enum State {
    On,
    Off,
}

#[bitmask(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Power {
    Exten = Self(1 << 0),
    DcDc3 = Self(1 << 1),
    Ldo2 = Self(1 << 2),
    Ldo4 = Self(1 << 3),
    DcDc2 = Self(1 << 4),
    Ldo3 = Self(1 << 6),
}

#[bitmask(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventsIrq {
    PowerKeyShortPress = Self(1 << 17),

    Int1 = Self(0xFF),
    Int2 = Self(0xFF00),
    Int3 = Self(0xFF0000),
    Int4 = Self(0xFF000000),
    Int5 = Self(0xFF00000000),
}

impl EventsIrq {
    fn is_int1(&self) -> bool {
        self.intersects(Self::Int1)
    }

    fn is_int2(&self) -> bool {
        self.intersects(Self::Int2)
    }

    fn is_int3(&self) -> bool {
        self.intersects(Self::Int3)
    }

    fn is_int4(&self) -> bool {
        self.intersects(Self::Int4)
    }

    fn is_int5(&self) -> bool {
        self.intersects(Self::Int5)
    }

    fn into_int1_u8(&self) -> u8 {
        let mask: u64 = self.bitand(Self::Int1).into();
        mask as u8
    }

    fn into_int2_u8(&self) -> u8 {
        let mask: u64 = self.bitand(Self::Int2).into();
        (mask >> 8) as u8
    }

    fn into_int3_u8(&self) -> u8 {
        let mask: u64 = self.bitand(Self::Int3).into();
        (mask >> 16) as u8
    }

    fn into_int4_u8(&self) -> u8 {
        let mask: u64 = self.bitand(Self::Int4).into();
        (mask >> 24) as u8
    }

    fn into_int5_u8(&self) -> u8 {
        let mask: u64 = self.bitand(Self::Int5).into();
        (mask >> 32) as u8
    }

    fn from_int1_u8(val: u8) -> Self {
        let mask: u64 = val as u64;
        Self::Int1.bitand(mask.into())
    }

    fn from_int2_u8(val: u8) -> Self {
        let mask: u64 = (val as u64)
            .checked_shl(8)
            .expect("Source being u8, this should not overflow");
        Self::Int2.bitand(mask.into())
    }

    fn from_int3_u8(val: u8) -> Self {
        let mask: u64 = (val as u64)
            .checked_shl(16)
            .expect("Source being u8, this should not overflow");
        Self::Int3.bitand(mask.into())
    }

    fn from_int4_u8(val: u8) -> Self {
        let mask: u64 = (val as u64)
            .checked_shl(24)
            .expect("Source being u8, this should not overflow");
        Self::Int4.bitand(mask.into())
    }

    fn from_int5_u8(val: u8) -> Self {
        let mask: u64 = (val as u64)
            .checked_shl(32)
            .expect("Source being u8, this should not overflow");
        Self::Int5.bitand(mask.into())
    }

    fn toggle(self, current_mask: EventsIrq, enable: bool) -> Self {
        if enable {
            self.bitor(current_mask)
        } else {
            self.bitand(!current_mask)
        }
    }
}

enum Register {
    Status = 0x00,
    IcType = 0x03,
    Ldo234Dc23Ctl = 0x12,
    EnabledIrq1 = 0x40,
    EnabledIrq2 = 0x41,
    EnabledIrq3 = 0x42,
    EnabledIrq4 = 0x43,
    EnabledIrq5 = 0x45,
    StatusIrq1 = 0x48,
    StatusIrq2 = 0x49,
    StatusIrq3 = 0x4A,
    StatusIrq4 = 0x4B,
    StatusIrq5 = 0x4C,
}

impl From<Register> for u8 {
    fn from(reg: Register) -> Self {
        reg as u8
    }
}

#[derive(Debug, Eq, PartialEq)]
enum ChipId {
    Unknown = 0x00,
    Axp202 = 0x41,
    Axp192 = 0x03,
    Axp173 = 0xAD,
}

impl From<u8> for ChipId {
    fn from(id: u8) -> Self {
        match id {
            0x41 => ChipId::Axp202,
            0x03 => ChipId::Axp192,
            0xAD => ChipId::Axp173,
            _ => ChipId::Unknown,
        }
    }
}

#[no_mangle]
#[inline(never)]
#[link_section = ".iram1"]
pub extern "C" fn axpxx_irq_triggered(_: *mut esp_idf_sys::c_types::c_void) {
    AXPXX_IRQ_TRIGGERED.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub struct Axpxx<I2C, SDA, SCL>
where
    I2C: I2c,
    SDA: OutputPin + InputPin,
    SCL: OutputPin,
{
    i2c: i2c::Master<I2C, SDA, SCL>,
    addr: u8,
    init: bool,
    chip_id: ChipId,
}

impl<I2C, SDA, SCL> Axpxx<I2C, SDA, SCL>
where
    I2C: I2c,
    SDA: OutputPin + InputPin,
    SCL: OutputPin,
{
    pub fn new(i2c: i2c::Master<I2C, SDA, SCL>) -> Self {
        Self {
            i2c,
            addr: AXP202_SLAVE_ADDR,
            init: false,
            chip_id: ChipId::Unknown,
        }
    }

    pub fn init(&mut self) -> Result<(), I2cError> {
        self.chip_id = self.probe_chip()?;

        self.init = true;

        Ok(())
    }

    pub fn init_irq(&mut self) -> Result<(), EspError> {
        let gpio_isr_config = esp_idf_sys::gpio_config_t {
            mode: GPIO_MODE_DEF_INPUT,
            pull_up_en: gpio_pullup_t_GPIO_PULLUP_DISABLE,
            pull_down_en: gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
            intr_type: gpio_int_type_t_GPIO_INTR_NEGEDGE,
            pin_bit_mask: 1 << GPIO_INTR,
        };
        unsafe {
            esp_idf_sys::rtc_gpio_deinit(GPIO_INTR.into());
            esp_idf_sys::gpio_config(&gpio_isr_config);

            esp_idf_sys::gpio_install_isr_service(0);
            esp_idf_sys::gpio_isr_handler_add(
                GPIO_INTR.into(),
                Some(axpxx_irq_triggered),
                std::ptr::null_mut(),
            );
        }

        Ok(())
    }

    fn read_reg(&mut self, reg: Register) -> Result<u8, I2cError> {
        let mut buf = [0u8; 1];
        let read_buf = [reg.into(); 1];
        self.i2c
            .write_read(self.addr, &read_buf, &mut buf)
            .and_then(|_| Ok(buf[0]))
    }

    fn write_reg(&mut self, reg: Register, val: u8) -> Result<(), I2cError> {
        self.i2c.write(self.addr, &[reg.into(), val])
    }

    fn probe_chip(&mut self) -> Result<ChipId, I2cError> {
        let chip_id = self.read_reg(Register::IcType)?;
        Ok(ChipId::from(chip_id))
    }

    pub fn toggle_irq(&mut self, irqs: EventsIrq, enable: bool) -> Result<(), I2cError> {
        if irqs.is_int1() {
            let irq1 = self.read_reg(Register::EnabledIrq1)?;
            let irq1 = EventsIrq::from_int1_u8(irq1);
            let irqs = irqs.toggle(irq1, enable);
            self.write_reg(Register::EnabledIrq1, irqs.into_int1_u8())?;
        }
        if irqs.is_int2() {
            let irq2 = self.read_reg(Register::EnabledIrq2)?;
            let irq2 = EventsIrq::from_int2_u8(irq2).bitor(irqs);
            let irqs = irqs.toggle(irq2, enable);
            self.write_reg(Register::EnabledIrq2, irqs.into_int2_u8())?;
        }
        if irqs.is_int3() {
            let irq3 = self.read_reg(Register::EnabledIrq3)?;
            let irq3 = EventsIrq::from_int3_u8(irq3).bitor(irqs);
            let irqs = irqs.toggle(irq3, enable);
            self.write_reg(Register::EnabledIrq3, irqs.into_int3_u8())?;
        }
        if irqs.is_int4() {
            let irq4 = self.read_reg(Register::EnabledIrq4)?;
            let irq4 = EventsIrq::from_int4_u8(irq4).bitor(irqs);
            let irqs = irqs.toggle(irq4, enable);
            self.write_reg(Register::EnabledIrq4, irqs.into_int4_u8())?;
        }
        if irqs.is_int5() {
            let irq5 = self.read_reg(Register::EnabledIrq5)?;
            let irq5 = EventsIrq::from_int5_u8(irq5).bitor(irqs);
            let irqs = irqs.toggle(irq5, enable);
            self.write_reg(Register::EnabledIrq5, irqs.into_int5_u8())?;
        }
        Ok(())
    }

    pub fn clear_irq(&mut self) -> Result<(), I2cError> {
        self.write_reg(Register::StatusIrq1, 0xFF)?;
        self.write_reg(Register::StatusIrq2, 0xFF)?;
        self.write_reg(Register::StatusIrq3, 0xFF)?;
        self.write_reg(Register::StatusIrq4, 0xFF)?;
        self.write_reg(Register::StatusIrq5, 0xFF)?;
        Ok(())
    }

    fn read_irq(&mut self) -> Result<EventsIrq, I2cError> {
        let irq1 = self.read_reg(Register::StatusIrq1)?;
        let irq2 = self.read_reg(Register::StatusIrq2)?;
        let irq3 = self.read_reg(Register::StatusIrq3)?;
        let irq4 = self.read_reg(Register::StatusIrq4)?;
        let irq5 = self.read_reg(Register::StatusIrq5)?;
        self.clear_irq()?;
        Ok(EventsIrq::from_int1_u8(irq1)
            .bitor(EventsIrq::from_int2_u8(irq2))
            .bitor(EventsIrq::from_int3_u8(irq3))
            .bitor(EventsIrq::from_int4_u8(irq4))
            .bitor(EventsIrq::from_int5_u8(irq5)))
    }

    pub fn is_button_pressed(&mut self) -> Result<bool, I2cError> {
        let is_irq_triggered = AXPXX_IRQ_TRIGGERED.load(Ordering::SeqCst);
        if is_irq_triggered {
            AXPXX_IRQ_TRIGGERED.store(false, Ordering::SeqCst);

            self.read_irq()
                .and_then(|irq| Ok(irq.intersects(EventsIrq::PowerKeyShortPress)))
        } else {
            Ok(false)
        }
    }

    pub fn debug_power_output(&mut self) -> Result<(), I2cError> {
        let data = self.read_reg(Register::Ldo234Dc23Ctl)?;
        println!("read 0b{:08b} before", data);
        Ok(())
    }

    pub fn set_power_output(
        &mut self,
        channel: Power,
        state: State,
        delay: &mut impl DelayMs<u32>,
    ) -> Result<(), I2cError> {
        // Before setting, the output cannot be all turned off
        let mut data: u8;
        loop {
            data = self.read_reg(Register::Ldo234Dc23Ctl)?;
            println!("read 0b{:08b} before", data);
            delay.delay_ms(10);
            if data != 0 {
                break;
            }
        }

        let mut data = Power::from(data);

        match state {
            State::On => {
                data |= channel;
            }
            State::Off => {
                data &= !channel;
            }
        };

        if self.chip_id == ChipId::Axp202 {
            data |= Power::DcDc3.into();
        }
        println!("About to write: {:?}", data);
        self.write_reg(Register::Ldo234Dc23Ctl, u8::from(data))?;
        delay.delay_ms(10);
        self.read_reg(Register::Ldo234Dc23Ctl).and_then(|data| {
            println!("read 0b{:08b} after", data);
            Ok(())
        })
    }
}
