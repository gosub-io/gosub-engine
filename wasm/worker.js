

import init, {wasm_execute_work} from "../pkg/"


console.log("Worker spawned")


await init()



self.onmessage = e => {
    // console.log("Worker", e.data)

    if (typeof e.data === "number") {
        // console.log("Received message: " + e.data)
        wasm_execute_work(e.data)
    }
    
    
    
}