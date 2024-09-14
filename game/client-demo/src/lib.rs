#![allow(clippy::too_many_arguments)]

pub mod ui;

use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{Read, Seek, Write},
    ops::RangeBounds,
    path::Path,
    rc::Rc,
    sync::{
        mpsc::{Receiver, Sender},
        Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

use anyhow::anyhow;
use av_encoder::encoder::{self, AudioVideoEncoder};
use base::{
    hash::Hash,
    reduced_ascii_str::ReducedAsciiString,
    system::{System, SystemTime, SystemTimeInterface},
};
use base_io::{io::Io, io_batcher::IoBatcherTask};
use client_map::client_map::{ClientMapFile, ClientMapLoading, GameMap};
use client_render_base::map::render_pipe::GameTimeInfo;
use client_render_game::render_game::{
    RenderForPlayer, RenderGameCreateOptions, RenderGameForPlayer, RenderGameInput,
    RenderGameInterface, RenderGameSettings, RenderPlayerCameraMode,
};
use client_ui::demo_player::user_data::{DemoViewerEvent, DemoViewerUiState, UserData};
use config::config::ConfigEngine;
use egui::Rect;
use game_config::config::ConfigMap;
use game_interface::{
    events::GameEvents,
    interface::{GameStateCreateOptions, GameStateInterface},
    types::{
        game::{GameTickType, NonZeroGameTickType},
        network_string::NetworkReducedAsciiString,
    },
};
use graphics::{
    graphics::graphics::Graphics,
    handles::{
        canvas::canvas::GraphicsCanvasHandle,
        stream::stream::{GraphicsStreamHandle, QuadStreamHandle},
        stream_types::StreamedQuad,
    },
};
use graphics_backend::backend::GraphicsBackend;
use graphics_types::{
    commands::CommandSwitchCanvasModeType,
    rendering::{BlendType, ColorMaskMode, State},
};
use hiarc::hi_closure;
use itertools::Itertools;
use math::math::vector::{ffixed, ubvec4, vec2};
use pool::datatypes::{PoolBTreeMap, PoolLinkedHashMap, PoolLinkedHashSet, PoolVec, PoolVecDeque};
use pool::mt_datatypes::PoolCow as MtPoolCow;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use shared_base::{
    game_types::intra_tick_time_to_ratio,
    network::{messages::GameModification, types::chat::NetChatMsg},
};
use sound::{
    commands::{SceneAirMode, SoundSceneCreateProps},
    sound::SoundManager,
};
use ui::render::{DemoPlayerUiRender, DemoPlayerUiRenderPipe};
use ui_base::{font_data::UiFontData, ui::UiCreator};

const DEMO_OFFSCREEN_ID: u64 = 9380;
const DEMO_VIDEO_ENCODER_OFFSCREEN_ID: u64 = 9_380_326;

pub type DemoGameModification = GameModification;

/// The demo header, of const size.
/// A broken demo can be detected if [`DemoHeader::len`] or
/// [`DemoHeader::size_chunks`] is zero.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct DemoHeader {
    /// Length of the full demo
    pub len: Duration,
    /// Size to read for the whole [`DemoHeaderExt`] struct.
    pub size_ext: u64,
    /// Size to read for all chunks.
    pub size_chunks: u64,
}

/// The tail of the demo is written last,
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DemoTail {
    /// the key is the monotonic tick, while the value is the
    /// file offset relative to the beginning of the chunk.
    snapshots_index: BTreeMap<u64, u64>,
    /// the key is the monotonic tick, while the value is the
    /// file offset relative to the beginning of the chunk.
    events_index: BTreeMap<u64, u64>,
}

/// A more flexible header, that can contain dynamic sized elements.
/// Here header simply means, never changing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemoHeaderExt {
    /// optional server name, address or whatever - can be left empty
    pub server: String,
    pub physics_mod: DemoGameModification,
    pub render_mod: DemoGameModification,
    /// resources the game **has** to load before
    /// the game/demo starts (e.g. because the game mod requires
    /// them for gameplay).
    pub required_resources: HashMap<String, String>,
    pub map: ReducedAsciiString,
    pub map_hash: Hash,
    pub ticks_per_second: NonZeroGameTickType,
    pub game_options: GameStateCreateOptions,
    pub physics_group_name: NetworkReducedAsciiString<24>,
}

/// When a chunk of snapshots or events ([`DemoRecorderChunk`]) is serialized, this header
/// is written.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChunkHeader {
    monotonic_tick: u64,
    size: u64,
}

pub type DemoSnapshot = Vec<u8>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DemoEvent {
    Game(GameEvents),
    Chat(NetChatMsg),
}

pub type DemoEvents = Vec<DemoEvent>;

enum DemoRecorderChunk {
    Snapshots { snaps: BTreeMap<u64, DemoSnapshot> },
    Events { events: BTreeMap<u64, DemoEvents> },
}

/// Records demos from snapshots & events
pub struct DemoRecorder {
    /// The dynamic sized header is only written once
    pub demo_header_ext: DemoHeaderExt,
    /// current demo snapshots
    pub snapshots: BTreeMap<u64, DemoSnapshot>,
    pub events: BTreeMap<u64, DemoEvents>,

    /// Event sender for the writer thread.
    /// Must stay to not be dropped
    thread_sender: Sender<DemoRecorderChunk>,
    /// the thread that writes all demo changes to disk
    _writer_thread: JoinHandle<()>,
}

// 50 here is the assumed snap send rate
// so it writes up to 30 seconds full of chunks
/// number of chunks to write at once
const DATA_PER_CHUNK_TO_WRITE: u64 = 30 * 50;
/// time offset so that even late packets have a chance
/// to be considered in the demo.
const SECONDS_UNTIL_WRITE: u64 = 3;

#[derive(Debug, Clone)]
pub struct DemoRecorderCreateProps {
    pub map: ReducedAsciiString,
    pub map_hash: Hash,
    pub game_options: GameStateCreateOptions,
    pub required_resources: HashMap<String, String>,
    pub physics_module: DemoGameModification,
    pub render_module: DemoGameModification,
    pub physics_group_name: NetworkReducedAsciiString<24>,
    pub io: Io,
}

impl DemoRecorder {
    pub fn new(
        props: DemoRecorderCreateProps,
        ticks_per_second: NonZeroGameTickType,
        mut forced_name: Option<String>,
    ) -> Self {
        let (thread_sender, recv) = std::sync::mpsc::channel();

        let now = chrono::Utc::now();
        let demo_name = forced_name
            .take()
            .unwrap_or_else(|| format!("{}_{}", props.map.as_str(), now.format("%Y_%m_%d_%H_%M")));

        let demo_header_ext = DemoHeaderExt {
            server: "".into(),
            physics_mod: props.physics_module,
            render_mod: props.render_module,
            required_resources: props.required_resources,
            map: props.map,
            map_hash: props.map_hash,
            ticks_per_second,
            game_options: props.game_options,
            physics_group_name: props.physics_group_name,
        };

        let io = props.io;

        let tmp_demo_dir = io.fs.get_save_path().join("tmp/demos");
        let demo_dir = io.fs.get_save_path().join("demos");
        let demo_header_ext_thread = demo_header_ext.clone();
        let writer_thread = std::thread::Builder::new()
            .name(format!("demo-recorder-{}", demo_header_ext.map.as_str()))
            .spawn(move || {
                Self::writer_thread_run(
                    &tmp_demo_dir,
                    &demo_dir,
                    &demo_name,
                    recv,
                    demo_header_ext_thread,
                )
                .unwrap()
            })
            .expect("could not spawn a demo-recorder thread.");

        Self {
            demo_header_ext,
            snapshots: Default::default(),
            events: Default::default(),

            thread_sender,
            _writer_thread: writer_thread,
        }
    }

    fn writer_thread_run(
        tmp_path: &Path,
        final_path: &Path,
        demo_name: &str,
        recv: Receiver<DemoRecorderChunk>,
        header_ext: DemoHeaderExt,
    ) -> anyhow::Result<()> {
        std::fs::create_dir_all(tmp_path)?;
        std::fs::create_dir_all(final_path)?;
        let mut tmp_file = tempfile::NamedTempFile::new_in(tmp_path)?;
        let file = tmp_file.as_file_mut();
        let size = Cell::new(0);

        fn ser_ex<'a, T: Serialize>(
            v: &T,
            writer: &'a mut Vec<u8>,
            clear: bool,
            fixed_size: bool,
        ) -> anyhow::Result<&'a mut [u8]> {
            if clear {
                writer.clear();
            }
            let config = bincode::config::standard();
            if fixed_size {
                bincode::serde::encode_into_std_write(v, writer, config.with_fixed_int_encoding())?;
            } else {
                bincode::serde::encode_into_std_write(v, writer, config)?;
            }
            Ok(writer.as_mut_slice())
        }
        fn ser<'a, T: Serialize>(v: &T, writer: &'a mut Vec<u8>) -> anyhow::Result<&'a mut [u8]> {
            ser_ex(v, writer, true, false)
        }

        fn comp<'a>(
            v: &[u8],
            writer: &'a mut Vec<u8>,
            clear_writer: bool,
        ) -> anyhow::Result<&'a mut [u8]> {
            if clear_writer {
                writer.clear();
            }
            let mut encoder = zstd::Encoder::new(&mut *writer, 0)?;
            encoder.write_all(v)?;
            encoder.finish()?;
            Ok(writer.as_mut_slice())
        }

        fn write(size: &Cell<usize>, file: &mut File, v: &[u8]) -> anyhow::Result<()> {
            size.set(size.get() + v.len());
            Ok(file.write_all(v)?)
        }

        let mut write_ser = Vec::new();
        let mut write_comp = Vec::new();
        let mut write_dst = Vec::new();
        let mut write_data = Vec::new();

        let header_ext_file = comp(ser(&header_ext, &mut write_ser)?, &mut write_comp, true)?;
        let header_ext_len = header_ext_file.len();

        write(
            &size,
            &mut *file,
            ser_ex(
                &DemoHeader {
                    len: Duration::ZERO,
                    size_ext: header_ext_len as u64,
                    // don't update this value before ending the demo
                    // makes it easy to detect corrupted demos
                    size_chunks: 0,
                },
                &mut write_ser,
                true,
                true,
            )?,
        )?;

        write(&size, &mut *file, header_ext_file)?;

        let mut first_monotonic_snaps = None;
        let mut last_monotonic_snaps = None;
        let mut first_monotonic_events = None;
        let mut last_monotonic_events = None;

        let mut events_index: BTreeMap<u64, u64> = Default::default();
        let mut snapshots_index: BTreeMap<u64, u64> = Default::default();

        let size_before_chunks = size.get();

        fn write_chunk<'a, A: Serialize>(
            chunk: BTreeMap<u64, A>,
            writer: &'a mut Vec<u8>,
            tmp: &mut Vec<u8>,
            tmp_dst: &mut Vec<u8>,
            tmp_patch_data: &mut Vec<u8>,
        ) -> anyhow::Result<&'a [u8]> {
            writer.clear();

            let mut last_data: Option<Vec<u8>> = None;

            // first write chunk count
            let len_ser = ser(&(chunk.len() as u64), &mut *tmp)?;
            writer.write_all(len_ser)?;

            for (monotonic_tick, data) in chunk {
                tmp_patch_data.clear();

                // prepare optimized data
                let data = {
                    let data_serialized = ser(&data, tmp_dst)?;
                    let data = if let Some(last_data) = &last_data {
                        bin_patch::diff(last_data, data_serialized, &mut *tmp_patch_data)?;
                        Some(tmp_patch_data.as_mut_slice())
                    } else {
                        Some(comp(data_serialized, tmp_patch_data, true)?)
                    };
                    last_data = Some(data_serialized.to_vec());
                    data
                };

                let mono_ser = ser(
                    &ChunkHeader {
                        monotonic_tick,
                        size: data.as_ref().map(|s| s.len() as u64).unwrap_or_default(),
                    },
                    &mut *tmp,
                )?;
                writer.write_all(mono_ser)?;
                // now write the data
                if let Some(data) = data {
                    writer.write_all(data)?;
                }
            }

            tmp_dst.clear();
            tmp_dst.extend(0_u64.to_le_bytes());
            comp(writer, tmp_dst, false)?;
            // write size
            let size = (tmp_dst.len() - std::mem::size_of::<u64>()) as u64;
            tmp_dst[0..std::mem::size_of::<u64>()].copy_from_slice(&size.to_le_bytes());
            std::mem::swap(writer, tmp_dst);
            Ok(writer.as_mut_slice())
        }

        #[allow(clippy::too_many_arguments)]
        fn serialize_and_write_chunk<A: Serialize>(
            file: &mut File,
            index: &mut BTreeMap<u64, u64>,
            chunk: BTreeMap<u64, A>,
            size: &Cell<usize>,
            size_before_chunks: usize,
            first_monotonic: &mut Option<u64>,
            last_monotonic: &mut Option<u64>,

            write_ser: &mut Vec<u8>,
            write_comp: &mut Vec<u8>,
            write_dst: &mut Vec<u8>,
            write_data: &mut Vec<u8>,
        ) -> anyhow::Result<()> {
            let first_tick = chunk
                .first_key_value()
                .map(|(c, _)| *c)
                .ok_or_else(|| anyhow!("empty chunks are not allowed."))?;

            let last_tick = chunk
                .last_key_value()
                .map(|(c, _)| *c)
                .ok_or_else(|| anyhow!("empty chunks are not allowed."))?;

            index.insert(first_tick, (size.get() - size_before_chunks) as u64);

            write(
                size,
                &mut *file,
                write_chunk(chunk, write_ser, write_comp, write_dst, write_data)?,
            )?;

            let monotonic_first_tick = *first_monotonic.get_or_insert(first_tick);
            anyhow::ensure!(
                monotonic_first_tick <= first_tick,
                "somehow the first recorded monotonic tick was bigger than the current first tick (so not monotonic)."
            );
            anyhow::ensure!(
                last_tick >= last_monotonic.unwrap_or_default(),
                "somehow the current last monotonic tick was smaller than the last one recorded (so not monotonic)."
            );
            let monotonic_last_tick = *last_monotonic.insert(last_tick);
            anyhow::ensure!(
                monotonic_last_tick >= monotonic_first_tick,
                "somehow the first monotonic tick was bigger than the current last one."
            );
            Ok(())
        }

        while let Ok(event) = recv.recv() {
            match event {
                DemoRecorderChunk::Snapshots { snaps } => {
                    serialize_and_write_chunk(
                        file,
                        &mut snapshots_index,
                        snaps,
                        &size,
                        size_before_chunks,
                        &mut first_monotonic_snaps,
                        &mut last_monotonic_snaps,
                        &mut write_ser,
                        &mut write_comp,
                        &mut write_dst,
                        &mut write_data,
                    )?;
                }
                DemoRecorderChunk::Events { events } => {
                    serialize_and_write_chunk(
                        file,
                        &mut events_index,
                        events,
                        &size,
                        size_before_chunks,
                        &mut first_monotonic_events,
                        &mut last_monotonic_events,
                        &mut write_ser,
                        &mut write_comp,
                        &mut write_dst,
                        &mut write_data,
                    )?;
                }
            }
        }

        let chunks_size = size.get() - size_before_chunks;

        // `or` to make sure None is never compared if there is one with Some
        // having Some is a must for the next if check
        let first_monotonic = (first_monotonic_snaps.or(first_monotonic_events))
            .min(first_monotonic_events.or(first_monotonic_snaps));
        let last_monotonic = (last_monotonic_snaps.or(last_monotonic_events))
            .max(last_monotonic_events.or(last_monotonic_snaps));

        if let Some((first_monotonic, last_monotonic)) = first_monotonic.zip(last_monotonic) {
            // write the demo tail
            write(
                &size,
                &mut *file,
                comp(
                    ser(
                        &DemoTail {
                            snapshots_index,
                            events_index,
                        },
                        &mut write_ser,
                    )?,
                    &mut write_comp,
                    true,
                )?,
            )?;

            // write the final header
            file.seek(std::io::SeekFrom::Start(0))?;
            file.write_all(ser_ex(
                &DemoHeader {
                    len: {
                        let secs = (last_monotonic - first_monotonic) / header_ext.ticks_per_second;
                        let nanos = ((last_monotonic - first_monotonic)
                            % header_ext.ticks_per_second)
                            * (Duration::from_secs(1).as_nanos() as u64
                                / header_ext.ticks_per_second);
                        Duration::new(secs, nanos as u32)
                    },
                    size_ext: header_ext_len as u64,
                    size_chunks: chunks_size as u64,
                },
                &mut write_ser,
                true,
                true,
            )?)?;

            file.flush()?;

            let (_, path) = tmp_file.keep()?;
            std::fs::rename(path, final_path.join(format!("{}.twdemo", demo_name)))?;
        }
        // else the demo is invalid and can be dropped.

        Ok(())
    }

    fn try_write_chunks<A>(
        data: &mut BTreeMap<u64, Vec<A>>,
        demo_header_ext: &DemoHeaderExt,
        thread_sender: &Sender<DemoRecorderChunk>,
        write: impl FnOnce(BTreeMap<u64, Vec<A>>) -> DemoRecorderChunk,
    ) {
        let get_chunks_th = || {
            let count = data.len();
            // find key as fast as possible
            if count >= (DATA_PER_CHUNK_TO_WRITE * 2) as usize {
                data.keys().nth(DATA_PER_CHUNK_TO_WRITE as usize)
            } else {
                data.keys()
                    .rev()
                    .nth((count - 1) - DATA_PER_CHUNK_TO_WRITE as usize)
            }
        };

        // check if chunks should be flushed
        // and the difference is > 3 seconds
        if data.len() > DATA_PER_CHUNK_TO_WRITE as usize
            && get_chunks_th()
                .copied()
                .zip(data.last_key_value().map(|(&tick, _)| tick))
                .is_some_and(|(first, last)| {
                    last - first > demo_header_ext.ticks_per_second.get() * SECONDS_UNTIL_WRITE
                })
        {
            let key = get_chunks_th();

            if let Some(&key) = key {
                let chunks = data.split_off(&key);
                // bit overcomplicated but split_off :/
                let chunks = std::mem::replace(data, chunks);

                // ignore the error here, if the write thread died, so be it, can't recover anyway.
                let _ = thread_sender.send(write(chunks));
            }
        }
    }

    fn can_add_chunk<A>(
        monotonic_tick: u64,
        data: &mut BTreeMap<u64, Vec<A>>,
        demo_header_ext: &DemoHeaderExt,
    ) -> bool {
        data.is_empty()
            || data.last_key_value().is_some_and(|(&key, _)| {
                monotonic_tick >= key
                    || (key - monotonic_tick)
                        <= demo_header_ext.ticks_per_second.get() * SECONDS_UNTIL_WRITE
            })
    }

    pub fn add_snapshot(&mut self, monotonic_tick: u64, snapshot: Vec<u8>) {
        Self::try_write_chunks(
            &mut self.snapshots,
            &self.demo_header_ext,
            &self.thread_sender,
            |snaps| DemoRecorderChunk::Snapshots { snaps },
        );

        // make sure only snapshots of the last 3 seconds are handled
        if Self::can_add_chunk(monotonic_tick, &mut self.snapshots, &self.demo_header_ext) {
            // if the entry already exist, update if, else create a new
            let entry = self.snapshots.entry(monotonic_tick).or_default();

            *entry = snapshot;
        }
    }

    pub fn add_event(&mut self, monotonic_tick: u64, event: DemoEvent) {
        Self::try_write_chunks(
            &mut self.events,
            &self.demo_header_ext,
            &self.thread_sender,
            |events| DemoRecorderChunk::Events { events },
        );

        // make sure only events of the last 3 seconds are handled
        if Self::can_add_chunk(monotonic_tick, &mut self.events, &self.demo_header_ext) {
            // if the entry already exist, update if, else create a new
            let entry = self.events.entry(monotonic_tick).or_default();

            entry.push(event);
        }
    }
}

impl Drop for DemoRecorder {
    fn drop(&mut self) {
        // write remaining chunks
        fn check_write<A>(
            data: &mut BTreeMap<u64, Vec<A>>,
            thread_sender: &Sender<DemoRecorderChunk>,
            write: impl Fn(BTreeMap<u64, Vec<A>>) -> DemoRecorderChunk,
        ) {
            if !data.is_empty() {
                std::mem::take(data)
                    .into_iter()
                    .chunks(DATA_PER_CHUNK_TO_WRITE as usize)
                    .into_iter()
                    .map(|chunks| chunks.collect::<BTreeMap<_, _>>())
                    .filter(|c| !c.is_empty())
                    .for_each(|chunks| {
                        // ignore the error here, if the write thread died, so be it, can't recover anyway.
                        let _ = thread_sender.send(write(chunks));
                    });
            }
        }
        check_write(&mut self.snapshots, &self.thread_sender, |snaps| {
            DemoRecorderChunk::Snapshots { snaps }
        });
        check_write(&mut self.events, &self.thread_sender, |events| {
            DemoRecorderChunk::Events { events }
        });
    }
}

#[derive(Debug, Clone)]
pub struct DemoContainer {
    /// The const sized header
    pub header: DemoHeader,
    /// Dynamic sized header
    pub header_ext: DemoHeaderExt,
    /// Demo chunks, still untouched (compressed, serialized)
    pub demo_chunks: Vec<u8>,
    pub tail: DemoTail,
}

#[derive(Debug)]
pub struct DemoViewerInner {
    demo: DemoContainer,
    cur_snapshots: BTreeMap<u64, DemoSnapshot>,
    cur_events: BTreeMap<u64, DemoEvents>,
    cur_time: Duration,
    is_closed: bool,
    is_paused: bool,
    speed: ffixed,
}

impl DemoViewerInner {
    pub fn is_finished(&self) -> bool {
        self.cur_time >= self.demo.header.len
    }

    pub fn is_closed(&self) -> bool {
        self.is_closed
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused
    }

    pub fn set_time_and_clear_chunks(&mut self, time: Duration) {
        self.cur_time = time;
        self.cur_snapshots.clear();
        self.cur_events.clear();
    }

    fn read_chunks<A: DeserializeOwned>(
        demo: &DemoContainer,
        offset: usize,
    ) -> anyhow::Result<BTreeMap<u64, A>> {
        let file = &demo.demo_chunks[offset..];

        fn deser<T: DeserializeOwned>(v: &[u8]) -> anyhow::Result<(T, usize)> {
            Ok(bincode::serde::decode_from_slice(
                v,
                bincode::config::standard(),
            )?)
        }
        fn decomp<'a>(v: &[u8], writer: &'a mut Vec<u8>) -> anyhow::Result<&'a [u8]> {
            writer.clear();
            let mut decoder = zstd::Decoder::new(v)?;
            decoder.read_to_end(&mut *writer)?;
            decoder.finish();

            Ok(writer.as_mut_slice())
        }

        // unpack all chunks
        let mut data: Vec<u8> = Default::default();

        anyhow::ensure!(
            file.len() >= std::mem::size_of::<u64>(),
            "file not huge enough to read u64 for compressed size"
        );
        let chunks_size = u64::from_le_bytes(file[0..std::mem::size_of::<u64>()].try_into()?);
        let file = &file[std::mem::size_of::<u64>()..];

        decomp(&file[0..chunks_size as usize], &mut data)?;
        let file = data.as_slice();

        // read item count in this chunk
        let (len, read_size) = deser::<u64>(file)?;

        let mut file = &file[read_size..];

        let mut res: BTreeMap<u64, A> = Default::default();

        let mut last_data: Option<Vec<u8>> = None;

        let mut writer: Vec<u8> = Default::default();

        for _ in 0..len {
            let (header, read_size) = deser::<ChunkHeader>(file)?;
            file = &file[read_size..];

            let data = if header.size > 0 {
                let data_slice = &file[..header.size as usize];
                let res = if let Some(last_data) = &last_data {
                    writer.clear();
                    bin_patch::patch(last_data, data_slice, &mut writer)?;
                    writer.as_slice()
                } else {
                    decomp(data_slice, &mut writer)?;
                    writer.as_slice()
                };
                last_data = Some(res.to_vec());
                let (data, _) = deser(res)?;
                file = &file[header.size as usize..];
                Some(data)
            } else {
                None
            };

            if let Some(data) = data {
                res.insert(header.monotonic_tick, data);
            }
        }

        Ok(res)
    }

    fn time_to_tick_impl(&self, time: Duration) -> u64 {
        (time.as_nanos()
            / (Duration::from_secs(1).as_nanos()
                / self.demo.header_ext.ticks_per_second.get() as u128)) as u64
            + (self
                .demo
                .tail
                .snapshots_index
                .first_key_value()
                .map(|(tick, _)| *tick)
                .unwrap_or(u64::MAX)
                .min(
                    self.demo
                        .tail
                        .events_index
                        .first_key_value()
                        .map(|(tick, _)| *tick)
                        .unwrap_or(u64::MAX),
                ))
    }

    fn time_to_tick(&self) -> u64 {
        self.time_to_tick_impl(self.cur_time)
    }

    fn intra_tick_time(
        &self,
        monotonic_tick: u64,
        prev_monotonic_tick: u64,
        next_monotonic_tick: u64,
    ) -> Duration {
        let tick_diff = next_monotonic_tick - prev_monotonic_tick;
        if tick_diff == 0 {
            Duration::ZERO
        } else {
            let nanos_per_tick = Duration::from_secs(1).as_nanos()
                / self.demo.header_ext.ticks_per_second.get() as u128;
            let time_in_tick = self.cur_time.as_nanos() % nanos_per_tick;
            (Duration::from_nanos(time_in_tick as u64)
                + Duration::from_nanos(
                    (monotonic_tick - prev_monotonic_tick) * nanos_per_tick as u64,
                ))
                / tick_diff as u32
        }
    }

    fn try_load_chunks<R: RangeBounds<u64>, A: DeserializeOwned>(
        demo: &DemoContainer,
        tick_range: R,
        rev: bool,
        cur_data: &mut BTreeMap<u64, A>,
        index: &BTreeMap<u64, u64>,
    ) {
        let mut it = index.range(tick_range);
        if let Some((_, chunk_byte_offset)) = if rev { it.next_back() } else { it.next() } {
            if let Ok(mut chunks) = Self::read_chunks::<A>(demo, *chunk_byte_offset as usize) {
                cur_data.append(&mut chunks);
            }
        }
    }

    fn check_chunks<A: DeserializeOwned>(
        demo: &DemoContainer,
        cur_data: &mut BTreeMap<u64, A>,
        index: &BTreeMap<u64, u64>,
        monotonic_tick: u64,
    ) {
        // try to load chunks if needed
        let first_tick = cur_data
            .first_key_value()
            .map_or(u64::MAX, |(&tick, _)| tick);
        if first_tick > monotonic_tick {
            Self::try_load_chunks(demo, ..=monotonic_tick, true, cur_data, index)
        }
        let last_tick = cur_data.last_key_value().map_or(0, |(&tick, _)| tick);
        if last_tick < monotonic_tick + 1 {
            Self::try_load_chunks(demo, last_tick + 1.., false, cur_data, index)
        }

        // we want exactly one chunk before the current tick
        // this allows the second chunks to be used for the tick
        // after this one.
        while cur_data.range(0..=monotonic_tick).count() > 1 {
            cur_data.pop_first();
        }
    }
}

pub struct DemoStaticData {
    canvas_handle: GraphicsCanvasHandle,
    stream_handle: GraphicsStreamHandle,

    av_encoder: Option<(AudioVideoEncoder, encoder::Receiver<Vec<u8>>)>,

    config_map: ConfigMap,
}

#[derive(Debug)]
pub struct DemoVideoEncodeProperties {}

pub struct DemoViewerImpl {
    data: DemoStaticData,

    client_map: ClientMapLoading,
    preview_client_map: ClientMapLoading,

    inner: DemoViewerInner,
    preview: DemoViewerInner,
    should_show_preview: Option<Rect>,

    demo_name: String,

    io: Io,

    time: Arc<SystemTime>,
    last_time: Option<Duration>,
    last_monotonic_tick: Option<GameTickType>,

    demo_ui: DemoPlayerUiRender,

    events: Vec<DemoViewerEvent>,
    ui_state: DemoViewerUiState,
}

impl DemoViewerImpl {
    fn new(
        client_map: ClientMapLoading,
        preview_client_map: ClientMapLoading,
        graphics: &Graphics,
        backend: &Rc<GraphicsBackend>,
        demo: DemoContainer,
        sys: &System,
        io: Io,
        ui_creator: &UiCreator,
        encode_to_video: Option<DemoVideoEncodeProperties>,
        name: String,
    ) -> Self {
        Self {
            data: DemoStaticData {
                canvas_handle: graphics.canvas_handle.clone(),
                stream_handle: graphics.stream_handle.clone(),
                av_encoder: encode_to_video.map(|_| {
                    let (sender, receiver) = encoder::channel();
                    (
                        AudioVideoEncoder::new(0, "test.webm", backend, sender).unwrap(),
                        receiver,
                    )
                }),
                config_map: Default::default(),
            },

            client_map,
            preview_client_map,

            demo_name: name,

            io: io.clone(),
            time: sys.time.clone(),
            last_time: None,
            last_monotonic_tick: None,

            preview: DemoViewerInner {
                demo: demo.clone(),

                cur_snapshots: Default::default(),
                cur_events: Default::default(),

                cur_time: Duration::ZERO,

                is_closed: false,
                // Always paused
                is_paused: true,
                speed: ffixed::from_num(1.0),
            },
            should_show_preview: None,
            inner: DemoViewerInner {
                demo,

                cur_snapshots: Default::default(),
                cur_events: Default::default(),

                cur_time: Duration::ZERO,

                is_closed: false,
                is_paused: false,
                speed: ffixed::from_num(1.0),
            },

            demo_ui: DemoPlayerUiRender::new(graphics, ui_creator),

            events: Default::default(),
            ui_state: Default::default(),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    fn set_time_and_reset_state(
        client_map: &mut ClientMapLoading,
        inner: &mut DemoViewerInner,
        time: Duration,
    ) {
        inner.set_time_and_clear_chunks(time);

        let map = client_map.try_get_mut().unwrap();

        let ClientMapFile::Game(GameMap { render, .. }) = map else {
            panic!("not a game map")
        };
        render.clear_render_state();
    }

    fn render_game(
        viewer: &mut DemoViewerInner,
        data: &mut DemoStaticData,
        client_map: &mut ClientMapLoading,
        cur_time: Duration,
        last_time: Duration,
        last_monotonic_tick: &mut Option<GameTickType>,
        for_video_encode: bool,
    ) -> anyhow::Result<()> {
        if !viewer.is_paused() && !viewer.is_finished() {
            viewer.cur_time += Duration::from_secs_f64(
                (cur_time.saturating_sub(last_time).as_secs_f64() * viewer.speed.to_num::<f64>())
                    .clamp(0.0, f64::MAX),
            );
        }

        let monotonic_tick = viewer.time_to_tick();

        DemoViewerInner::check_chunks(
            &viewer.demo,
            &mut viewer.cur_snapshots,
            &viewer.demo.tail.snapshots_index,
            monotonic_tick,
        );
        DemoViewerInner::check_chunks(
            &viewer.demo,
            &mut viewer.cur_events,
            &viewer.demo.tail.events_index,
            monotonic_tick,
        );

        let map = client_map.try_get_mut().unwrap();

        let ClientMapFile::Game(GameMap { render, game }) = map else {
            panic!("not a game map")
        };

        let Some((local_players, prev_tick, next_tick)) = (if !viewer.cur_snapshots.is_empty() {
            let mut it = viewer.cur_snapshots.iter();

            if let Some((&prev_tick, snap)) = it.next() {
                let mut snapshot = MtPoolCow::new_without_pool();
                snapshot.to_mut().extend(snap.iter());
                let local_players = game.build_from_snapshot(&snapshot);
                let next_tick = if let Some((&next_tick, snap)) = it.next() {
                    let old_snapshot = snapshot;
                    let mut snapshot = MtPoolCow::new_without_pool();
                    snapshot.to_mut().extend(snap.iter());
                    game.build_from_snapshot_for_pred(&old_snapshot, &snapshot);
                    next_tick
                } else {
                    prev_tick
                };
                Some((local_players, prev_tick, next_tick))
            } else {
                None
            }
        } else {
            None
        }) else {
            return Err(anyhow!("no snapshots inside this demo"));
        };

        let (player_id, _) = local_players.iter().next().unwrap();
        let intra_tick_time = viewer.intra_tick_time(monotonic_tick, prev_tick, next_tick);

        let render_for_player = RenderForPlayer {
            chat_info: None,
            emote_wheel_input: None,
            scoreboard_active: false,
            chat_show_all: false,

            local_player_info: game.collect_character_local_render_info(player_id),

            zoom: 1.0,
            cam_mode: RenderPlayerCameraMode::Default,
        };

        let game_time_info = GameTimeInfo {
            ticks_per_second: game.game_tick_speed(),
            intra_tick_time,
        };
        let intra_tick_ratio =
            intra_tick_time_to_ratio(intra_tick_time, viewer.demo.header_ext.ticks_per_second);

        let character_infos = game.collect_characters_info();

        let stages = game.all_stages(intra_tick_ratio);

        let scoreboard_info = None; // game.collect_scoreboard_info();

        let load_events = !last_monotonic_tick.is_some_and(|tick| tick == monotonic_tick);
        last_monotonic_tick.replace(monotonic_tick);
        let (events, chat_msgs) = if load_events {
            let demo_events = viewer
                .cur_events
                .get(&monotonic_tick)
                .cloned()
                .unwrap_or_default();

            let mut events = PoolBTreeMap::new_without_pool();
            let mut chat_msgs = PoolVecDeque::new_without_pool();
            for demo_event in demo_events {
                match demo_event {
                    DemoEvent::Game(evs) => {
                        events.insert(monotonic_tick, (evs, false));
                    }
                    DemoEvent::Chat(msg) => {
                        chat_msgs.push_back(msg);
                    }
                }
            }
            (events, chat_msgs)
        } else {
            (
                PoolBTreeMap::new_without_pool(),
                PoolVecDeque::new_without_pool(),
            )
        };

        let mut render_game_input = RenderGameInput {
            players: PoolLinkedHashMap::new_without_pool(),
            dummies: PoolLinkedHashSet::new_without_pool(),
            events,
            vote: None,
            character_infos,
            stages,
            scoreboard_info,
            chat_msgs,
            game_time_info,
            settings: RenderGameSettings {
                // TODO: add config for demos
                sound_playback_speed: viewer.speed.to_num::<f64>(),
                spartial_sound: false,
                map_sound_volume: 0.3,
                ingame_sound_volume: 0.3,
                nameplates: true,
                nameplate_own: false,
            },
        };

        let render_for_player = RenderGameForPlayer {
            render_for_player,
            observed_players: PoolVec::new_without_pool(),
            observed_anchored_size_props: Default::default(),
        };

        render_game_input
            .players
            .insert(*player_id, render_for_player);

        if for_video_encode {
            data.canvas_handle
                .switch_canvas(CommandSwitchCanvasModeType::Offscreen {
                    id: 0,
                    width: 800,
                    height: 600,
                    has_multi_sampling: None,
                    pixels_per_point: 1.0,
                });
        }

        render.render(&data.config_map, &viewer.cur_time, render_game_input);

        if for_video_encode {
            data.canvas_handle
                .switch_canvas(CommandSwitchCanvasModeType::Onscreen);
        }

        Ok(())
    }

    pub fn render(&mut self, input: egui::RawInput) -> anyhow::Result<()> {
        let cur_time = self.time.time_get_nanoseconds();
        let last_time = self.last_time.replace(cur_time).unwrap_or(cur_time);
        let do_encoding = self.data.av_encoder.is_some();
        Self::render_game(
            &mut self.inner,
            &mut self.data,
            &mut self.client_map,
            cur_time,
            last_time,
            &mut self.last_monotonic_tick,
            do_encoding,
        )?;
        self.demo_ui.render(
            &mut DemoPlayerUiRenderPipe {
                cur_time: &self.time.time_get_nanoseconds(),
                player_info: UserData {
                    stream_handle: &self.data.stream_handle,
                    canvas_handle: &self.data.canvas_handle,
                    is_paused: &self.inner.is_paused(),
                    cur_duration: &self.inner.cur_time,
                    max_duration: &self.inner.demo.header.len,
                    speed: &self.inner.speed,
                    events: &mut self.events,
                    state: &mut self.ui_state,
                    name: &self.demo_name,
                },
            },
            input,
        );

        self.should_show_preview = None;

        // handle events after demo
        for event in self.events.drain(..) {
            match event {
                DemoViewerEvent::ResumeToggle => {
                    self.inner.is_paused = !self.inner.is_paused;
                }
                DemoViewerEvent::Stop => {
                    self.inner.is_paused = true;
                    Self::set_time_and_reset_state(
                        &mut self.client_map,
                        &mut self.inner,
                        Duration::ZERO,
                    );
                }
                DemoViewerEvent::BackwardFast => {
                    Self::set_time_and_reset_state(
                        &mut self.client_map,
                        &mut self.inner,
                        Duration::ZERO,
                    );
                }
                DemoViewerEvent::ForwardFast => {
                    let time = self.inner.demo.header.len;
                    Self::set_time_and_reset_state(&mut self.client_map, &mut self.inner, time);
                }
                DemoViewerEvent::BackwardStep => {
                    self.inner.cur_time = self.inner.cur_time.saturating_sub(Duration::from_nanos(
                        (Duration::from_secs(1).as_nanos()
                            / self.inner.demo.header_ext.ticks_per_second.get() as u128)
                            as u64,
                    ));
                }
                DemoViewerEvent::ForwardStep => {
                    self.inner.cur_time = self.inner.cur_time.saturating_add(Duration::from_nanos(
                        (Duration::from_secs(1).as_nanos()
                            / self.inner.demo.header_ext.ticks_per_second.get() as u128)
                            as u64,
                    ));
                }
                DemoViewerEvent::Backward => todo!(),
                DemoViewerEvent::Forward => todo!(),
                DemoViewerEvent::SpeedSlower => {
                    self.inner.speed /= ffixed::from_num(2);
                    self.inner.speed = self
                        .inner
                        .speed
                        .clamp(ffixed::from_num(0.03125), ffixed::from_num(4096.0));
                }
                DemoViewerEvent::SpeedFaster => {
                    self.inner.speed *= ffixed::from_num(2);
                    self.inner.speed = self
                        .inner
                        .speed
                        .clamp(ffixed::from_num(0.03125), ffixed::from_num(4096.0));
                }
                DemoViewerEvent::SpeedReset => {
                    self.inner.speed = ffixed::from_num(1.0);
                }
                DemoViewerEvent::Export(data) => {
                    let demo = &self.inner.demo;
                    let ext = &demo.header_ext;
                    let mut recorder = DemoRecorder::new(
                        DemoRecorderCreateProps {
                            map: ext.map.clone(),
                            map_hash: ext.map_hash,
                            game_options: ext.game_options.clone(),
                            required_resources: ext.required_resources.clone(),
                            physics_module: ext.physics_mod.clone(),
                            render_module: ext.render_mod.clone(),
                            physics_group_name: ext.physics_group_name.clone(),
                            io: self.io.clone(),
                        },
                        ext.ticks_per_second,
                        Some(data.name),
                    );
                    self.preview.set_time_and_clear_chunks(data.left);

                    let last_monotonic_tick = self.preview.time_to_tick_impl(data.right);
                    let mut monotonic_tick = self.preview.time_to_tick();

                    while monotonic_tick <= last_monotonic_tick {
                        DemoViewerInner::check_chunks(
                            &self.preview.demo,
                            &mut self.preview.cur_snapshots,
                            &self.preview.demo.tail.snapshots_index,
                            monotonic_tick,
                        );
                        DemoViewerInner::check_chunks(
                            &self.preview.demo,
                            &mut self.preview.cur_events,
                            &self.preview.demo.tail.events_index,
                            monotonic_tick,
                        );

                        if let Some(snapshot) = self.preview.cur_snapshots.get(&monotonic_tick) {
                            recorder.add_snapshot(monotonic_tick, snapshot.clone());
                        }
                        if let Some(events) = self.preview.cur_events.get(&monotonic_tick) {
                            for event in events
                                .iter()
                                .filter(|ev| !matches!(ev, DemoEvent::Chat(_)) || !data.remove_chat)
                            {
                                recorder.add_event(monotonic_tick, event.clone());
                            }
                        }

                        monotonic_tick += 1;
                    }
                }
                DemoViewerEvent::SkipTo { time } => {
                    Self::set_time_and_reset_state(&mut self.client_map, &mut self.inner, time);
                }
                DemoViewerEvent::PreviewAt { rect, time } => {
                    Self::set_time_and_reset_state(
                        &mut self.preview_client_map,
                        &mut self.preview,
                        time,
                    );
                    self.should_show_preview = Some(rect);
                }
                DemoViewerEvent::Close => {
                    self.inner.is_closed = true;
                }
            }
        }

        if let Some(rect) = self.should_show_preview {
            self.data
                .canvas_handle
                .switch_canvas(CommandSwitchCanvasModeType::Offscreen {
                    id: DEMO_OFFSCREEN_ID,
                    width: rect.width() as u32,
                    height: rect.height() as u32,
                    pixels_per_point: 0.25,
                    has_multi_sampling: None,
                });

            Self::render_game(
                &mut self.preview,
                &mut self.data,
                &mut self.preview_client_map,
                Duration::ZERO,
                Duration::ZERO,
                &mut Default::default(),
                false,
            )?;
            self.data
                .canvas_handle
                .switch_canvas(CommandSwitchCanvasModeType::Onscreen);

            let mut state = State::new();
            state.map_canvas(
                0.0,
                0.0,
                self.data.canvas_handle.canvas_width(),
                self.data.canvas_handle.canvas_height(),
            );
            state.set_color_mask(ColorMaskMode::WriteColorOnly);
            state.blend(BlendType::None);

            let rect = &rect;
            self.data.stream_handle.render_quads(
                hi_closure!([
                    rect: &Rect,
                ], |mut stream_handle: QuadStreamHandle<'_>| -> () {
                    stream_handle.set_offscreen_attachment_texture(DEMO_OFFSCREEN_ID);
                    stream_handle
                        .add_vertices(
                            StreamedQuad::default().from_pos_and_size(
                                vec2::new(
                                    rect.left_top().x,
                                    rect.left_top().y
                                ),
                                vec2::new(rect.width(), rect.height())
                            )
                            .color(
                                ubvec4::new(255, 255, 255, 255)
                            )
                            .tex_free_form(
                                vec2::new(0.0, 0.0),
                                vec2::new(1.0, 0.0),
                                vec2::new(1.0, 1.0),
                                vec2::new(0.0, 1.0),
                            ).into()
                        );
                }),
                state,
            );
        }

        // video encoding finished.
        if self.data.av_encoder.is_some() && self.is_finished() {
            self.inner.is_closed = true;
        }

        Ok(())
    }
}

impl Drop for DemoViewerImpl {
    fn drop(&mut self) {
        if let Some((encoder, file)) = self.data.av_encoder.take() {
            drop(encoder);
            let fs = self.io.fs.clone();
            self.io.io_batcher.spawn_without_lifetime(async move {
                let file = file.await?;
                fs.write_file("testtest.webm".as_ref(), file).await?;
                Ok(())
            });
        }
    }
}

pub struct DemoViewerLoading {
    pub task: IoBatcherTask<DemoContainer>,
    pub io: Io,
    pub thread_pool: Arc<rayon::ThreadPool>,
    pub fonts: Arc<UiFontData>,
    encode_to_video: Option<DemoVideoEncodeProperties>,
    name: String,
}

pub struct DemoViewerLoadingComponents {
    pub client_map: ClientMapLoading,
    pub preview_client_map: ClientMapLoading,
    pub demo: DemoContainer,
    pub io: Io,
    encode_to_video: Option<DemoVideoEncodeProperties>,
    name: String,
}

pub enum DemoViewer {
    Loading(Box<DemoViewerLoading>),
    LoadingComponents(Box<DemoViewerLoadingComponents>),
    Rendering(Box<DemoViewerImpl>),
    None,
}

impl DemoViewer {
    pub fn new(
        io: &Io,
        thread_pool: &Arc<rayon::ThreadPool>,
        demo_path: &Path,
        fonts: Arc<UiFontData>,
        encode_to_video: Option<DemoVideoEncodeProperties>,
    ) -> Self {
        let fs = io.fs.clone();
        let demo_path_thread = demo_path.to_path_buf();
        let read_demo = io.io_batcher.spawn(async move {
            let demo = fs.read_file(&demo_path_thread).await?;

            fn decomp<'a>(v: &[u8], writer: &'a mut Vec<u8>) -> anyhow::Result<&'a [u8]> {
                writer.clear();
                let mut decoder = zstd::Decoder::new(v)?;
                decoder.read_to_end(&mut *writer)?;
                decoder.finish();

                Ok(writer.as_mut_slice())
            }
            fn deser_ex<T: DeserializeOwned>(
                v: &[u8],
                fixed_size: bool,
            ) -> anyhow::Result<(T, usize)> {
                if fixed_size {
                    Ok(bincode::serde::decode_from_slice(
                        v,
                        bincode::config::standard().with_fixed_int_encoding(),
                    )?)
                } else {
                    Ok(bincode::serde::decode_from_slice(
                        v,
                        bincode::config::standard(),
                    )?)
                }
            }
            fn deser<T: DeserializeOwned>(v: &[u8]) -> anyhow::Result<(T, usize)> {
                deser_ex(v, false)
            }

            let mut writer: Vec<u8> = Default::default();

            // read header
            let (header, file_off): (DemoHeader, usize) = deser_ex(&demo, true)?;
            let demo = &demo[file_off..];

            // read header ext
            let (header_ext, _): (DemoHeaderExt, usize) =
                deser(decomp(&demo[0..header.size_ext as usize], &mut writer)?)?;

            let demo = &demo[header.size_ext as usize..];
            let chunks = &demo[0..header.size_chunks as usize];
            let tail = &demo[header.size_chunks as usize..];

            // read tail
            let (tail, _): (DemoTail, usize) = deser(decomp(tail, &mut writer)?)?;
            anyhow::ensure!(
                !tail.snapshots_index.is_empty(),
                "no snapshot index found in demo tail."
            );

            // read all chunks
            Ok(DemoContainer {
                header,
                header_ext,
                demo_chunks: chunks.to_vec(),
                tail,
            })
        });
        Self::Loading(Box::new(DemoViewerLoading {
            task: read_demo,
            io: io.clone(),
            thread_pool: thread_pool.clone(),
            fonts,
            encode_to_video,
            name: demo_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        }))
    }

    pub fn try_get(&self) -> Option<&DemoViewerImpl> {
        if let Self::Rendering(viewer) = self {
            Some(viewer)
        } else {
            None
        }
    }

    pub fn try_get_mut(&mut self) -> Option<&mut DemoViewerImpl> {
        if let Self::Rendering(viewer) = self {
            Some(viewer)
        } else {
            None
        }
    }

    pub fn continue_loading(
        &mut self,
        sound: &SoundManager,
        graphics: &Graphics,
        backend: &Rc<GraphicsBackend>,
        config: &ConfigEngine,
        sys: &System,
        ui_creator: &UiCreator,
    ) -> anyhow::Result<Option<&DemoViewerImpl>> {
        let mut dummy = DemoViewer::None;
        std::mem::swap(self, &mut dummy);
        match dummy {
            DemoViewer::Loading(loading) => {
                let DemoViewerLoading {
                    task,
                    io,
                    thread_pool,
                    fonts,
                    encode_to_video,
                    name,
                } = *loading;
                if task.is_finished() {
                    let demo_container = task.get_storage()?;
                    let gen_client_map = |sound_props: SoundSceneCreateProps| {
                        ClientMapLoading::new(
                            "map/maps/".as_ref(),
                            &demo_container.header_ext.map,
                            Some(demo_container.header_ext.map_hash),
                            &io,
                            &thread_pool,
                            demo_container.header_ext.physics_mod.clone(),
                            false,
                            demo_container.header_ext.game_options.clone(),
                            RenderGameCreateOptions {
                                physics_group_name: demo_container
                                    .header_ext
                                    .physics_group_name
                                    .clone(),
                                resource_download_server: None,
                                fonts: fonts.clone(),
                                sound_props,
                            },
                        )
                    };
                    *self = Self::LoadingComponents(Box::new(DemoViewerLoadingComponents {
                        client_map: gen_client_map(if encode_to_video.is_some() {
                            SoundSceneCreateProps {
                                air_mode: SceneAirMode::OffAir {
                                    id: DEMO_VIDEO_ENCODER_OFFSCREEN_ID,
                                    sample_rate: 48000,
                                },
                            }
                        } else {
                            SoundSceneCreateProps::default()
                        }),
                        preview_client_map: gen_client_map(SoundSceneCreateProps {
                            air_mode: SceneAirMode::OffAir {
                                id: DEMO_OFFSCREEN_ID,
                                sample_rate: 48000,
                            },
                        }),
                        demo: demo_container,
                        io,
                        encode_to_video,
                        name,
                    }));
                } else {
                    *self = Self::Loading(Box::new(DemoViewerLoading {
                        task,
                        io,
                        thread_pool,
                        fonts,
                        encode_to_video,
                        name,
                    }));
                }
            }
            DemoViewer::LoadingComponents(loading) => {
                let DemoViewerLoadingComponents {
                    mut client_map,
                    mut preview_client_map,
                    demo,
                    io,
                    encode_to_video,
                    name,
                } = *loading;
                if client_map
                    .continue_loading(sound, graphics, backend, config, sys)
                    .is_some()
                    && preview_client_map
                        .continue_loading(sound, graphics, backend, config, sys)
                        .is_some()
                {
                    // finished loading client
                    *self = Self::Rendering(Box::new(DemoViewerImpl::new(
                        client_map,
                        preview_client_map,
                        graphics,
                        backend,
                        demo,
                        sys,
                        io,
                        ui_creator,
                        encode_to_video,
                        name,
                    )));
                } else {
                    *self = Self::LoadingComponents(Box::new(DemoViewerLoadingComponents {
                        client_map,
                        preview_client_map,
                        demo,
                        io,
                        encode_to_video,
                        name,
                    }));
                }
            }
            DemoViewer::Rendering(viewer) => {
                *self = Self::Rendering(viewer);
            }
            DemoViewer::None => {}
        }
        Ok(self.try_get())
    }
}
