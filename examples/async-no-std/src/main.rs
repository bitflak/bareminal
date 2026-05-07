#![no_std]
#![no_main]

use core::mem::MaybeUninit;

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::exti::{self};
use embassy_stm32::peripherals::{self, DMA1_CH1, DMA1_CH2, PD8, PD9, USART3};
use embassy_stm32::usart::{self, Uart};
use embassy_stm32::{Peri, SharedData, bind_interrupts, dma, interrupt};
use {defmt_rtt as _, panic_probe as _};

use heapless::String;
use serde::{Deserialize, Serialize};

use bareminal_cli::cli::{Bareminal, CommandWriter};
use bareminal_macros::{Command, CommandGroup};

#[unsafe(link_section = ".sram4.shared_data")]
static SHARED_DATA: MaybeUninit<SharedData> = MaybeUninit::uninit();

bind_interrupts!(pub struct Irqs{
    EXTI15_10 => exti::InterruptHandler<interrupt::typelevel::EXTI15_10>;
    USART3 => usart::InterruptHandler<peripherals::USART3>;
    DMA1_STREAM1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_STREAM2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hsi = Some(HSIPrescaler::DIV1);
        config.rcc.csi = true;
        config.rcc.pll1 = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV4,
            mul: PllMul::MUL50,
            divp: Some(PllDiv::DIV2),
            divq: Some(PllDiv::DIV8), // 100mhz
            divr: None,
        });
        config.rcc.pll2 = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV4,
            mul: PllMul::MUL50,
            divp: Some(PllDiv::DIV2),
            divq: Some(PllDiv::DIV8), // 100mhz
            divr: None,
        });
        config.rcc.sys = Sysclk::PLL1_P; // 400 Mhz
        config.rcc.ahb_pre = AHBPrescaler::DIV2; // 200 Mhz
        config.rcc.apb1_pre = APBPrescaler::DIV2; // 100 Mhz
        config.rcc.apb2_pre = APBPrescaler::DIV2; // 100 Mhz
        config.rcc.apb3_pre = APBPrescaler::DIV2; // 100 Mhz
        config.rcc.apb4_pre = APBPrescaler::DIV2; // 100 Mhz
        config.rcc.voltage_scale = VoltageScale::Scale1;
        config.rcc.supply_config = SupplyConfig::DirectSMPS;
    }

    let p = embassy_stm32::init_primary(config, &SHARED_DATA);

    // let now = Instant::now();

    spawner.spawn(unwrap!(usart_task(
        p.USART3, p.PD9, p.PD8, p.DMA1_CH1, p.DMA1_CH2
    )));

    info!("Setup complete");
    // let elapsed = now.elapsed();
}

/// General command description.
/// Multiline also possible.
#[derive(Debug, Command)]
enum BaseCommands<'a> {
    /// Help for individual command
    /// In the help overview, only the first line is visible
    /// CamelCase name will be transformed into kebab-case
    SimpleCommand,
    /// You can set default and min and max values
    #[set(default = 2, min = -5, max = 3)]
    Int(i32),
    /// All primitive types are supported
    #[set(min = -1.0, max = 2.0)]
    Float(f32),
    /// Min and max values are also possible for chars
    #[set(default = 'a', max = 'c')]
    Char(char),
    /// You can define a set of possible values by one_of attribute
    #[set(default = 'a', one_of = ['a', 'b'])]
    Set(char),
    /// Compound values are possible too
    #[set(default = (42, 42), min = (1,1), max = (42,42))]
    Compound((i32, i32)),
    /// You can print optionally enumerated lists
    List,
    /// You can print tables
    Table,
    /// You can print or pretty print json values directly
    /// Uses serde and serde_json_core under the hood
    SomeJson {
        #[set(default = Some(false))]
        json: Option<bool>,
        #[set(default = Some(false))]
        pretty: Option<bool>,
    },
    /// Enum values are possible too, but you have to
    /// implement FromStr trait. A bit cumbersome, but
    /// it also gives you some flexibility.
    #[set(default = "mode1", one_of = ["mode1", "mode2"])]
    Modes(Option<Modes>),
    /// Command can also have flags.
    /// Use struct like enum variants for this
    Command {
        /// Same attributes are possible
        /// additionally you can set short attribute
        /// if set, short flag version -n will be possible
        #[set(default = "Bareminal", short)]
        name: &'a str,
        /// If short version collides with something else
        /// you can define a custom version
        #[set(default = "is awesome", short = 'm')]
        name2: &'a str,
        /// Snake case is again transformed to kebab-case.
        /// On boolean flags you can omit the value so
        /// just --some-flag is same as --some-flag true
        #[set(default = Some(false))]
        some_flag: Option<bool>,
    },
    /// Async version also provides a special loop method.
    /// See the documentation for more information
    Loop,
}

/// Some second command group
#[derive(Debug, Command)]
enum SecondCommandGroup {
    /// When two commands have a similar name,
    /// autocomplete will pick the shorter one.
    Simple2,
}

/// Doc comments on CommandGroup will be ignored
#[derive(Debug, CommandGroup)]
enum CommandGroup<'a> {
    Base(BaseCommands<'a>),
    Second(SecondCommandGroup),
}

#[derive(Serialize, Deserialize)]
struct Nested {
    meta: u8,
}

#[derive(Serialize, Deserialize)]
struct SomeJson {
    pin: u8,
    value: u8,
    nested: Nested,
}

#[derive(Debug, PartialEq)]
enum Modes {
    Mode1,
    Mode2,
}

use core::str::FromStr;

impl FromStr for Modes {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mode1" => Ok(Modes::Mode1),
            "mode2" => Ok(Modes::Mode2),
            _ => Err(()),
        }
    }
}

const MAX_CMD_BUFFER: usize = 256;
const HISTORY_SIZE: usize = 3;

#[embassy_executor::task]
async fn usart_task(
    peri: Peri<'static, USART3>,
    rx_pin: Peri<'static, PD9>,
    tx_pin: Peri<'static, PD8>,
    tx_dma: Peri<'static, DMA1_CH1>,
    rx_dma: Peri<'static, DMA1_CH2>,
) {
    let usart_config = embassy_stm32::usart::Config::default();

    if let Ok(uart) = Uart::new(peri, rx_pin, tx_pin, tx_dma, rx_dma, Irqs, usart_config) {
        let (tx, rx) = uart.split();
        let mut cli =
            match Bareminal::<CommandGroup, _, MAX_CMD_BUFFER, HISTORY_SIZE>::new(tx).await {
                Ok(cli) => cli,
                Err(err) => {
                    error!("{}", err);
                    return;
                }
            };

        let buf = unsafe {
            static mut BUF: [u8; 256] = [0u8; 256];
            &mut *core::ptr::addr_of_mut!(BUF)
        };

        let mut rx = rx.into_ring_buffered(buf);

        let mut read_buf = [0u8; 64];

        loop {
            match rx.read(&mut read_buf[..]).await {
                Ok(received) => {
                    'outer: for &byte in &read_buf[..received] {
                        if let Ok(ready) = cli
                            .add_byte(byte)
                            .await
                            .inspect_err(|err| error!("{}", err))
                            && ready
                        {
                            loop {
                                let result = cli.next_command().await;
                                match result {
                                    Ok(None) => {
                                        if let Err(err) = cli.finalize().await {
                                            error!("{}", err);
                                            break 'outer;
                                        }
                                        break;
                                    }
                                    Ok(Some((command, writer))) => {
                                        process_command(command, writer, &mut rx).await;
                                    }
                                    Err(err) => {
                                        error!("{}", err);
                                        break 'outer;
                                    }
                                };
                            }
                        }
                    }
                }
                Err(err) => {
                    error!("{}", err);
                    break;
                }
            }
        }
    }
}

async fn process_command<
    'a,
    W: embedded_io_async::Write + Unpin,
    R: embedded_io_async::Read + Unpin + ?Sized,
>(
    command: CommandGroup<'a>,
    writer: &'a mut CommandWriter<W>,
    reader: &'a mut R,
) {
    let some_json = SomeJson {
        pin: 42,
        value: 42,
        nested: Nested { meta: 42 },
    };

    match command {
        CommandGroup::Base(base_command) => match base_command {
            BaseCommands::SimpleCommand => {
                let s: String<64> = heapless::format!("simple command").unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
            BaseCommands::Int(value) => {
                let s: String<64> = heapless::format!("{:?}", value).unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
            BaseCommands::Float(value) => {
                let s: String<64> = heapless::format!("{:?}", value).unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
            BaseCommands::Char(value) => {
                let s: String<64> = heapless::format!("{:?}", value).unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
            BaseCommands::Set(value) => {
                let s: String<64> = heapless::format!("{:?}", value).unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
            BaseCommands::Compound((value1, value2)) => {
                let s: String<64> = heapless::format!("{:?}, {:?}", value1, value2).unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
            BaseCommands::List => {
                let rows: &[&[&str]] = &[&["Pin A", "30"], &["Pin B", "42"]];
                let _ = writer.write_list::<256>(rows, true).await;
            }
            BaseCommands::Table => {
                let headers = ["Name", "Value"];
                let rows: &[&[&str]] = &[&["Pin A", "30"], &["Pin B", "42"]];
                let _ = writer.write_table::<256>(&headers, rows).await;
            }
            BaseCommands::SomeJson { json, pretty } => {
                if let Some(true) = json {
                    let _ = writer
                        .write_json::<256>(&some_json, pretty.unwrap_or(false))
                        .await;
                } else {
                    let s: String<64> =
                        heapless::format!("(pin: {}, value: {})", some_json.pin, some_json.value)
                            .unwrap();
                    let _ = writer.write_line(s.as_bytes()).await;
                };
            }
            BaseCommands::Command {
                name,
                name2,
                some_flag,
            } => {
                let s: String<128> =
                    heapless::format!("{:?} {:?} {:?}", name, name2, some_flag).unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
            BaseCommands::Modes(value) => {
                let s: String<64> = heapless::format!("{:?}", value).unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
            BaseCommands::Loop => {
                let _ = writer
                    .write_loop::<128>(reader, async move |mut buf| {
                        let headers = ["Name", "Value"];
                        let rows: &[&[&str]] = &[&["Pin A", "30"], &["Pin B", "42"]];
                        buf.write_table(&headers, rows).unwrap();
                        buf
                    })
                    .await;
            }
        },
        CommandGroup::Second(command) => match command {
            SecondCommandGroup::Simple2 => {
                let s: String<64> = heapless::format!("simple2 command").unwrap();
                let _ = writer.write_line(s.as_bytes()).await;
            }
        },
    };
}
