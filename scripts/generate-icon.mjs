// Original geometric crystal; no external artwork or image dependencies.
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { deflateSync } from 'node:zlib';

const size = 64;
const pixels = Buffer.alloc(size * size * 4);
for (let y = 0; y < size; y++) {
  for (let x = 0; x < size; x++) {
    const dx = (x + 0.5 - 32) / 21;
    const dy = (y + 0.5 - 32) / 29;
    const inside = Math.abs(dx) + Math.abs(dy) < 1;
    const color = inside ? (dx < 0 ? [196, 226, 213, 255] : [109, 164, 141, 255]) : [32, 58, 50, 255];
    pixels.set(color, (y * size + x) * 4);
  }
}

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}
function chunk(type, bytes) {
  const data = Buffer.concat([Buffer.from(type), bytes]);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(bytes.length);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(data));
  return Buffer.concat([length, data, crc]);
}
const header = Buffer.alloc(13);
header.writeUInt32BE(size, 0);
header.writeUInt32BE(size, 4);
header[8] = 8;
header[9] = 6;
const scanlines = Buffer.concat(Array.from({ length: size }, (_, row) => Buffer.concat([
  Buffer.from([0]), pixels.subarray(row * size * 4, (row + 1) * size * 4),
])));
const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header),
  chunk('IDAT', deflateSync(scanlines)), chunk('IEND', Buffer.alloc(0)),
]);
const iconHeader = Buffer.alloc(22);
iconHeader.writeUInt16LE(1, 2);
iconHeader.writeUInt16LE(1, 4);
iconHeader[6] = size;
iconHeader[7] = size;
iconHeader.writeUInt16LE(1, 10);
iconHeader.writeUInt16LE(32, 12);
iconHeader.writeUInt32LE(png.length, 14);
iconHeader.writeUInt32LE(22, 18);
const directory = new URL('../apps/desktop/src-tauri/icons/', import.meta.url);
mkdirSync(directory, { recursive: true });
writeFileSync(new URL('icon.png', directory), png);
writeFileSync(new URL('icon.ico', directory), Buffer.concat([iconHeader, png]));
console.log(`Generated original icons in ${fileURLToPath(directory)}`);
