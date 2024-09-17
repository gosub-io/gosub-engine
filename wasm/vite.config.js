import {defineConfig} from "vite";

export default defineConfig({
    server: {
        fs: {
            allow: ['../pkg', "./"]
        },
        headers: {
            // for SharedArrayBuffer
            'Cross-Origin-Opener-Policy': 'same-origin',
            'Cross-Origin-Embedder-Policy': 'require-corp'
        }
    }
})