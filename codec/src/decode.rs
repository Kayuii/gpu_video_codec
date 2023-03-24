use crate::get_bin_file;
use common::{
    CodecID::{self, *},
    DecodeCalls, HWDeviceType, PixelFormat,
    PixelFormat::NV12,
    MAX_DATA_NUM,
};
use log::{error, trace};
use std::{
    ffi::c_void,
    os::raw::c_int,
    slice::from_raw_parts,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};
use DecodeDriver::*;

pub struct Decoder {
    calls: DecodeCalls,
    codec: Box<c_void>,
    frames: *mut Vec<DecodeFrame>,
    pub ctx: DecodeContext,
}

unsafe impl Send for Decoder {}
unsafe impl Sync for Decoder {}

impl Decoder {
    pub fn new(ctx: DecodeContext) -> Result<Self, ()> {
        let calls = match ctx.driver {
            CUVID => nvidia::decode_calls(),
            AMF => amf::decode_calls(),
        };
        unsafe {
            let codec = (calls.new)(
                ctx.device as i32,
                ctx.pixfmt as i32,
                ctx.codec as i32,
                ctx.gpu,
            );
            if codec.is_null() {
                return Err(());
            }
            Ok(Self {
                calls,
                codec: Box::from_raw(codec as *mut c_void),
                frames: Box::into_raw(Box::new(Vec::<DecodeFrame>::new())),
                ctx,
            })
        }
    }

    pub fn decode(&mut self, packet: &[u8]) -> Result<&mut Vec<DecodeFrame>, i32> {
        unsafe {
            (&mut *self.frames).clear();
            let ret = (self.calls.decode)(
                &mut *self.codec,
                packet.as_ptr() as _,
                packet.len() as _,
                Some(Self::callback),
                self.frames as *mut _ as *mut c_void,
            );

            if ret < 0 {
                error!("Error decode: {}", ret);
                Err(ret)
            } else {
                Ok(&mut *self.frames)
            }
        }
    }

    unsafe extern "C" fn callback(
        datas: *mut *mut u8,
        linesizes: *mut i32,
        format: c_int,
        width: c_int,
        height: c_int,
        obj: *const c_void,
        key: c_int,
    ) {
        let frames = &mut *(obj as *mut Vec<DecodeFrame>);
        let datas = from_raw_parts(datas, MAX_DATA_NUM as _);
        let linesizes = from_raw_parts(linesizes, MAX_DATA_NUM as _);

        let mut frame = DecodeFrame {
            pixfmt: std::mem::transmute(format),
            width,
            height,
            data: vec![],
            linesize: vec![],
            key: key != 0,
        };

        if format == PixelFormat::NV12 as c_int {
            let y = from_raw_parts(datas[0], (linesizes[0] * height) as usize).to_vec();
            let uv = from_raw_parts(datas[1], (linesizes[1] * height / 2) as usize).to_vec();

            frame.data.push(y);
            frame.data.push(uv);

            frame.linesize.push(linesizes[0]);
            frame.linesize.push(linesizes[1]);

            frames.push(frame);
        } else {
            error!("unsupported pixfmt {}", format);
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            (self.calls.destroy)(self.codec.as_mut());
            let _ = Box::from_raw(self.frames);
            trace!("Decoder dropped");
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodeDriver {
    CUVID,
    AMF,
}

#[derive(Debug, Clone)]
pub struct DecodeContext {
    pub driver: DecodeDriver,
    pub device: HWDeviceType,
    pub pixfmt: PixelFormat,
    pub codec: CodecID,
    pub gpu: i32,
}

pub struct DecodeFrame {
    pub pixfmt: PixelFormat,
    pub width: i32,
    pub height: i32,
    pub data: Vec<Vec<u8>>,
    pub linesize: Vec<i32>,
    pub key: bool,
}

impl std::fmt::Display for DecodeFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::from("data:");
        for data in self.data.iter() {
            s.push_str(format!("{} ", data.len()).as_str());
        }
        s.push_str(", linesize:");
        for linesize in self.linesize.iter() {
            s.push_str(format!("{} ", linesize).as_str());
        }

        write!(
            f,
            "surface_format:{:?}, width:{}, height:{},key:{}, {}",
            self.pixfmt, self.width, self.height, self.key, s,
        )
    }
}

pub fn available() -> Vec<DecodeContext> {
    use std::sync::Once;
    static mut CACHED: Vec<DecodeContext> = vec![];
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        CACHED = available_();
    });
    unsafe { CACHED.clone() }
}

fn available_() -> Vec<DecodeContext> {
    // to-do: log control
    let gpu = 0;
    let format = NV12;
    let mut natives: Vec<_> = nvidia::possible_support_decoders()
        .drain(..)
        .map(|n| (CUVID, n))
        .collect();
    natives.append(
        &mut amf::possible_support_decoders()
            .drain(..)
            .map(|n| (AMF, n))
            .collect(),
    );
    let inputs = natives.drain(..).map(|(driver, n)| DecodeContext {
        driver,
        device: n.device,
        pixfmt: format,
        codec: n.codec,
        gpu,
    });
    let outputs = Arc::new(Mutex::new(Vec::<DecodeContext>::new()));
    let mut p_bin_264: *mut u8 = std::ptr::null_mut();
    let mut len_bin_264: c_int = 0;
    let buf264;
    let mut p_bin_265: *mut u8 = std::ptr::null_mut();
    let mut len_bin_265: c_int = 0;
    let buf265;
    unsafe {
        get_bin_file(0, &mut p_bin_264 as _, &mut len_bin_264 as _);
        get_bin_file(1, &mut p_bin_265 as _, &mut len_bin_265 as _);
        buf264 = from_raw_parts(p_bin_264, len_bin_264 as _);
        buf265 = from_raw_parts(p_bin_265, len_bin_265 as _);
    }
    let buf264 = Arc::new(buf264);
    let buf265 = Arc::new(buf265);
    // let mut handles = vec![];
    for ctx in inputs {
        let outputs = outputs.clone();
        let buf264 = buf264.clone();
        let buf265 = buf265.clone();
        // let handle = thread::spawn(move || {
        let start = Instant::now();
        if let Ok(mut decoder) = Decoder::new(ctx.clone()) {
            log::debug!("{:?} new:{:?}", ctx, start.elapsed());
            let data = match ctx.codec {
                H264 => &buf264[..],
                HEVC => &buf265[..],
                _ => continue,
            };
            let start = Instant::now();
            if let Ok(_) = decoder.decode(data) {
                log::debug!("{:?} decode:{:?}", ctx, start.elapsed());
                outputs.lock().unwrap().push(ctx);
            } else {
                log::debug!("{:?} decode failed:{:?}", ctx, start.elapsed());
            }
        } else {
            log::debug!("{:?} new failed:{:?}", ctx, start.elapsed());
        }
        // });

        // handles.push(handle);
    }
    // for handle in handles {
    //     handle.join().ok();
    // }
    let x = outputs.lock().unwrap().clone();
    x
}
