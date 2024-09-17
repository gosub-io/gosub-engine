use std::fmt::format;
use std::future::Future;
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread::yield_now;
use anyhow::anyhow;
use js_sys::{Array, Atomics, SharedArrayBuffer};
use js_sys::wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::{console, Worker, WorkerOptions, WorkerType};
use log::info;
use web_sys::console::info;
use crate::types::Result;

pub struct WasmWorker {
    worker: Worker,
}


pub struct Work {
    f: Box<dyn FnOnce()>,
}


impl WasmWorker {
    pub fn new() -> Result<Self> {


        let opts = WorkerOptions::new();
        opts.set_type(WorkerType::Module);
        
        
        
        let undefined = JsValue::UNDEFINED;
        
        

        let worker = Worker::new_with_options("worker.js", &opts)
            .map_err(|e| anyhow!("{:?}", e))?;


        info!("Worker created (main)");



        // let array = Array::new();
        // 
        // array.push(&wasm_bindgen::module());
        // array.push(&wasm_bindgen::memory());
        // 
        // worker.post_message(&array)
        //     .map_err(|e| {console::log_1(&e); e})
        //     .map_err(|e| anyhow!("{:?}", e))
        //     .map_err(|e|  {info!("Error sending module and memory to worker: {:?}", e); e})?;
        // 
        // 
        // 
        // info!("Sending module and memory to worker");


        Ok(Self {
            worker,
        })






    }

    pub fn execute(&mut self, f: impl FnOnce() + Send + 'static) -> Result<()> {
        let work = Box::new(Work {
            f: Box::new(f),
        });

        let ptr = Box::into_raw(work) as *mut _ as usize;

        self.worker.post_message(&JsValue::from(ptr))
            .map_err(|e| anyhow!("{:?}", e))?;

        Ok(())
    }
    
    
    
    

    pub fn execute_blocking(&mut self, f: impl FnOnce() + Send + 'static) -> Result<()> {
        let block = Arc::new(AtomicBool::new(false));

        let block_clone = block.clone();

        let closure = move || {
            f();

            block_clone.store(true, Ordering::Relaxed);
        };


        let work = Box::new(Work {
            f: Box::new(closure),
        });

        let ptr = Box::into_raw(work) as *mut _ as usize;

        info!("Posting message");
        self.worker.post_message(&JsValue::from(ptr))
            .map_err(|e| anyhow!("{:?}", e))?;


        while block.load(Ordering::Relaxed) {
            std::hint::spin_loop();
        }

        Ok(())
    }


    pub fn execute_blocking_return<R: Send + 'static>(&mut self, f: impl FnOnce() -> R + Send + 'static) -> Result<R> {
        let res = Arc::new(Mutex::new(None));
        let block = Arc::new(AtomicU8::new(0));

        // 0 = block
        // 1 = finished
        // 2 = lock error


        let block_clone = block.clone();
        let res_clone = res.clone();

        let closure = move || {
            let res = f();

            let Ok(mut lock) = res_clone.lock() else {
                block_clone.store(2, Ordering::Relaxed);
                return;
            };

            *lock = Some(res);


            block_clone.store(1, Ordering::Relaxed);
        };


        let work = Box::new(Work {
            f: Box::new(closure),
        });

        let ptr = Box::into_raw(work) as *mut _ as usize;

        info!("Posting message");
        self.worker.post_message(&JsValue::from(ptr))
            .map_err(|e| anyhow!("{:?}", e))?;


        while block.load(Ordering::Relaxed) == 0 {
            std::hint::spin_loop();
        }

        if block.load(Ordering::Relaxed) == 2 {
            return Err(anyhow!("Lock error"));
        }


        let Ok(mut lock) = res.lock() else {
            return Err(anyhow!("Lock error"));
        };

        let Some(res) = lock.take() else {
            return Err(anyhow!("Worker did not return"));
        };

        Ok(res)
    }


    pub fn async_blocking<F: Future<Output=()> + 'static>(&mut self, f: F) -> Result<()> {
        let block = Arc::new(AtomicBool::new(false));

        let block_clone = block.clone();

        let closure = move || {
            wasm_bindgen_futures::spawn_local(async move {
                f.await;

                block_clone.store(true, Ordering::Relaxed);
            })
        };


        let work = Box::new(Work {
            f: Box::new(closure),
        });

        let ptr = Box::into_raw(work) as *mut _ as usize;

        info!("Posting message");
        self.worker.post_message(&JsValue::from(ptr))
            .map_err(|e| anyhow!("{:?}", e))?;

        Ok(())
    }
    pub fn async_blocking_return<F: Future + 'static>(&mut self, f: F) -> Result<F::Output> {


        // 0 = block
        // 1 = finished
        // 2 = lock error


        let block = Arc::new(AtomicU8::new(0));
        let ret = Arc::new(Mutex::new(None));


        let block_clone = block.clone();
        let ret_clone = ret.clone();

        let closure = move || {
            wasm_bindgen_futures::spawn_local(async move {
                let ret = f.await;

                let Ok(mut lock) = ret_clone.lock() else {
                    block_clone.store(2, Ordering::Relaxed);


                    return;
                };

                *lock = Some(ret);


                block_clone.store(1, Ordering::Relaxed);

            })
        };


        let work = Box::new(Work {
            f: Box::new(closure),
        });

        let ptr = Box::into_raw(work) as *mut _ as usize;

        info!("Posting message");
        self.worker.post_message(&JsValue::from(ptr))
            .map_err(|e| anyhow!("{:?}", e))?;




        while block.load(Ordering::Relaxed) == 0 {
            yield_now();
        }

        if block.load(Ordering::Relaxed) == 2 {
            return Err(anyhow!("Lock error"));
        }

        let Ok(mut lock) = ret.lock() else {
            return Err(anyhow!("Lock error"));
        };

        let Some(ret) = lock.take() else {
            return Err(anyhow!("Worker did not return"));
        };

        Ok(ret)
    }
}



#[wasm_bindgen]
pub fn wasm_execute_work(ptr: usize) {
    let work = unsafe { Box::from_raw(ptr as *mut Work) };
    (work.f)();
}
