import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";

const ROOT_DIR = path.resolve(".");
const BRAND_SVG = path.resolve(ROOT_DIR, "../../assets/clipx-brand-mark-squircle.svg");
const OUT_DIR = path.join(ROOT_DIR, "src-tauri", "windows", "images");

const BLUE = "#3B82F6";
const VIOLET = "#8B5CF6";
const WHITE = "#FFFFFF";

const gradientSvg = (w, h) => `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}">
  <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="${BLUE}"/><stop offset="1" stop-color="${VIOLET}"/>
  </linearGradient></defs>
  <rect width="${w}" height="${h}" fill="url(#g)"/>
</svg>`;

/** Encode raw top-down RGB pixels as an uncompressed 24-bit BMP (sharp cannot write BMP). */
function toBmp(data, w, h) {
  const rowSize = Math.ceil((w * 3) / 4) * 4;
  const out = Buffer.alloc(54 + rowSize * h);
  out.write("BM", 0);
  out.writeUInt32LE(out.length, 2);
  out.writeUInt32LE(54, 10);
  out.writeUInt32LE(40, 14);
  out.writeInt32LE(w, 18);
  out.writeInt32LE(h, 22);
  out.writeUInt16LE(1, 26);
  out.writeUInt16LE(24, 28);
  out.writeUInt32LE(rowSize * h, 34);
  out.writeInt32LE(2835, 38);
  out.writeInt32LE(2835, 42);
  for (let y = 0; y < h; y++) {
    const dst = 54 + (h - 1 - y) * rowSize; // BMP rows are stored bottom-up
    for (let x = 0; x < w; x++) {
      const s = (y * w + x) * 3;
      out[dst + x * 3] = data[s + 2];
      out[dst + x * 3 + 1] = data[s + 1];
      out[dst + x * 3 + 2] = data[s];
    }
  }
  return out;
}

async function save(name, pipeline, w, h) {
  const data = await pipeline.flatten({ background: WHITE }).removeAlpha().raw().toBuffer();
  fs.writeFileSync(path.join(OUT_DIR, name), toBmp(data, w, h));
  console.log(`  ✔ ${name} (${w}x${h})`);
}

export async function makeInstallerImages() {
  console.log("🖼  Generating Windows installer images from the brand mark...");
  if (!fs.existsSync(BRAND_SVG)) {
    console.error(`  ✖ Brand mark not found at ${BRAND_SVG}`);
    process.exit(1);
  }
  fs.mkdirSync(OUT_DIR, { recursive: true });

  const brand = fs.readFileSync(BRAND_SVG, "utf8");
  // White paperclip only (no squircle, no shadow) for the gradient side panels.
  const clipSvg = brand.replace(/<rect[^>]*\/>/, "").replace(' filter="url(#drop-shadow)"', "");

  const mark = (size) => sharp(Buffer.from(brand), { density: 384 }).resize(size, size).png().toBuffer();

  const panel = async (w, h) => {
    const size = Math.round(w * 0.92);
    const clip = await sharp(Buffer.from(clipSvg), { density: 384 }).resize(size, size).png().toBuffer();
    return sharp(Buffer.from(gradientSvg(w, h)))
      .composite([{ input: clip, left: Math.round((w - size) / 2), top: Math.round((h - size) / 2) }])
      .png()
      .toBuffer();
  };

  const white = (w, h) => sharp({ create: { width: w, height: h, channels: 3, background: WHITE } });

  // NSIS welcome/finish sidebar
  await save("nsis-sidebar.bmp", sharp(await panel(164, 314)), 164, 314);

  // NSIS page header: white, mark on the right
  await save("nsis-header.bmp", white(150, 57).composite([{ input: await mark(46), left: 150 - 46 - 6, top: 5 }]), 150, 57);

  // MSI welcome/finish dialog: brand panel on the left, white elsewhere
  await save("msi-dialog.bmp", white(493, 312).composite([{ input: await panel(164, 312), left: 0, top: 0 }]), 493, 312);

  // MSI banner: white, mark on the right, thin brand strip along the bottom
  const strip = await sharp(Buffer.from(gradientSvg(493, 3))).png().toBuffer();
  await save(
    "msi-banner.bmp",
    white(493, 58).composite([
      { input: await mark(48), left: 493 - 48 - 10, top: 5 },
      { input: strip, left: 0, top: 55 },
    ]),
    493,
    58,
  );
}