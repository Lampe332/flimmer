// Writes a minimal 16x16 32-bit icon.ico (tauri-build needs one on Windows).
import { writeFileSync } from 'node:fs'
const out = process.argv[2]
const w = 16, h = 16
const xor = Buffer.alloc(w * h * 4)
for (let i = 0; i < w * h; i++) xor.writeUInt32LE(0xffe0a020, i * 4) // BGRA
const and = Buffer.alloc(((w + 31) >> 5) * 4 * h)
const bih = Buffer.alloc(40)
bih.writeUInt32LE(40, 0); bih.writeInt32LE(w, 4); bih.writeInt32LE(h * 2, 8)
bih.writeUInt16LE(1, 12); bih.writeUInt16LE(32, 14); bih.writeUInt32LE(xor.length + and.length, 20)
const img = Buffer.concat([bih, xor, and])
const dir = Buffer.alloc(6 + 16)
dir.writeUInt16LE(1, 2); dir.writeUInt16LE(1, 4)
dir[6] = w; dir[7] = h; dir.writeUInt16LE(1, 10); dir.writeUInt16LE(32, 12)
dir.writeUInt32LE(img.length, 14); dir.writeUInt32LE(22, 18)
writeFileSync(out, Buffer.concat([dir, img]))
console.log('wrote', out, 22 + img.length)
