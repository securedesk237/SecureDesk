const sharp = require('sharp');
const fs = require('fs');
const path = require('path');

const svgContent = `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
  <defs>
    <linearGradient id="shieldGradient" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#3b82f6;stop-opacity:1" />
      <stop offset="100%" style="stop-color:#1d4ed8;stop-opacity:1" />
    </linearGradient>
  </defs>
  <circle cx="256" cy="256" r="240" fill="url(#shieldGradient)"/>
  <path d="M256 96 L128 144 L128 272 C128 368 256 432 256 432 C256 432 384 368 384 272 L384 144 Z"
        fill="white" stroke="none"/>
  <rect x="216" y="240" width="80" height="64" rx="8" fill="#3b82f6"/>
  <path d="M232 240 L232 216 C232 189 256 176 256 176 C256 176 280 189 280 216 L280 240"
        stroke="#3b82f6" stroke-width="16" fill="none" stroke-linecap="round"/>
  <circle cx="256" cy="272" r="12" fill="white"/>
  <rect x="252" y="272" width="8" height="20" fill="white"/>
</svg>`;

const iconsDir = path.join(__dirname, 'src-tauri', 'icons');

async function generateIcons() {
  // Ensure icons directory exists
  if (!fs.existsSync(iconsDir)) {
    fs.mkdirSync(iconsDir, { recursive: true });
  }

  const svgBuffer = Buffer.from(svgContent);

  // Generate PNG icons
  const sizes = [
    { name: '32x32.png', size: 32 },
    { name: '128x128.png', size: 128 },
    { name: '128x128@2x.png', size: 256 },
    { name: 'icon.png', size: 512 },
  ];

  for (const { name, size } of sizes) {
    await sharp(svgBuffer)
      .resize(size, size)
      .png()
      .toFile(path.join(iconsDir, name));
    console.log(`Generated ${name}`);
  }

  // Generate ICO for Windows (256x256)
  const icoBuffer = await sharp(svgBuffer)
    .resize(256, 256)
    .png()
    .toBuffer();

  // For ICO, we'll create a simple PNG-based ICO
  // ICO format: header + directory entries + image data
  const pngData = icoBuffer;
  const icoHeader = Buffer.alloc(6);
  icoHeader.writeUInt16LE(0, 0); // Reserved
  icoHeader.writeUInt16LE(1, 2); // Type: 1 = ICO
  icoHeader.writeUInt16LE(1, 4); // Number of images

  const dirEntry = Buffer.alloc(16);
  dirEntry.writeUInt8(0, 0);  // Width (0 = 256)
  dirEntry.writeUInt8(0, 1);  // Height (0 = 256)
  dirEntry.writeUInt8(0, 2);  // Color palette
  dirEntry.writeUInt8(0, 3);  // Reserved
  dirEntry.writeUInt16LE(1, 4);  // Color planes
  dirEntry.writeUInt16LE(32, 6); // Bits per pixel
  dirEntry.writeUInt32LE(pngData.length, 8); // Size of image data
  dirEntry.writeUInt32LE(22, 12); // Offset to image data (6 + 16)

  const icoFile = Buffer.concat([icoHeader, dirEntry, pngData]);
  fs.writeFileSync(path.join(iconsDir, 'icon.ico'), icoFile);
  console.log('Generated icon.ico');

  // For macOS ICNS, we'll create a placeholder (proper ICNS generation is complex)
  // Tauri can work with just the PNG files on macOS build
  await sharp(svgBuffer)
    .resize(512, 512)
    .png()
    .toFile(path.join(iconsDir, 'icon.icns.png'));

  // Create a simple ICNS file (this is a simplified version)
  // For production, use a proper ICNS generator
  fs.copyFileSync(
    path.join(iconsDir, 'icon.png'),
    path.join(iconsDir, 'icon.icns')
  );
  console.log('Generated icon.icns (placeholder - use proper tool for macOS)');

  console.log('\nAll icons generated successfully!');
}

generateIcons().catch(console.error);
