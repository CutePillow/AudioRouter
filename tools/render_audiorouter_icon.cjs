const fs = require('node:fs/promises');
const path = require('node:path');
const sharp = require('C:/Users/17739/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/sharp');

async function main() {
  const output = path.resolve(__dirname, '../Self-disk/AudioRouter-icon-v1');
  const source = path.resolve(__dirname, '../assets/icons/audiorouter-mark.svg');
  await fs.mkdir(output, { recursive: true });
  await fs.copyFile(source, path.join(output, 'audiorouter.svg'));
  for (const size of [16, 24, 32, 48, 64, 128, 256, 512, 1024]) {
    await sharp(source, { density: 384 }).resize(size, size).png().toFile(path.join(output, `audiorouter-${size}.png`));
  }
  const rawIcon = await sharp(source, { density: 384 }).resize(64, 64).ensureAlpha().raw().toBuffer();
  await fs.writeFile(path.join(output, 'audiorouter-icon-64.rgba'), rawIcon);
  await fs.copyFile(path.join(output, 'audiorouter-icon-64.rgba'), path.resolve(__dirname, '../assets/icons/audiorouter-icon-64.rgba'));
  const sizes = [16, 24, 32, 48, 64, 128, 256];
  const buffers = await Promise.all(sizes.map(size => fs.readFile(path.join(output, `audiorouter-${size}.png`))));
  const header = Buffer.alloc(6 + sizes.length * 16);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((size, index) => {
    const entry = 6 + index * 16;
    header[entry] = size === 256 ? 0 : size;
    header[entry + 1] = size === 256 ? 0 : size;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(buffers[index].length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += buffers[index].length;
  });
  await fs.writeFile(path.join(output, 'audiorouter.ico'), Buffer.concat([header, ...buffers]));
  const vector = `<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="108dp" android:height="108dp" android:viewportWidth="108" android:viewportHeight="108">
  <group android:scaleX="0.16" android:scaleY="0.16" android:translateX="13.04" android:translateY="13.04">
    <path android:pathData="M102,198C147,137 211,141 256,219c45,78 109,82 154,21" android:fillColor="#00000000" android:strokeColor="#96EBC6" android:strokeWidth="28" android:strokeLineCap="round"/>
    <path android:pathData="M410,314C365,375 301,371 256,293c-45,-78 -109,-82 -154,-21" android:fillColor="#00000000" android:strokeColor="#F5BB4F" android:strokeWidth="28" android:strokeLineCap="round"/>
    <path android:pathData="M199,256a57,57 0,1 0,114 0a57,57 0,1 0,-114 0" android:fillColor="#173E39" android:strokeColor="#D8FFEC" android:strokeWidth="7"/>
    <path android:pathData="M236,247v18M256,235v42M276,247v18" android:fillColor="#00000000" android:strokeColor="#FFD98A" android:strokeWidth="9" android:strokeLineCap="round"/>
  </group>
</vector>\n`;
  const android = path.join(output, 'android');
  await fs.mkdir(android, { recursive: true });
  await fs.writeFile(path.join(android, 'ic_launcher_foreground.xml'), vector);
  await fs.writeFile(path.join(android, 'ic_launcher.xml'), '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android"><background android:drawable="@drawable/ic_launcher_background"/><foreground android:drawable="@drawable/ic_launcher_foreground"/></adaptive-icon>\n');
  await fs.writeFile(path.join(android, 'ic_launcher_background.xml'), '<shape xmlns:android="http://schemas.android.com/apk/res/android" android:shape="rectangle"><solid android:color="#173E39"/></shape>\n');
  await fs.copyFile(path.join(output, 'audiorouter-512.png'), path.resolve(__dirname, '../assets/icons/audiorouter-icon.png'));
  await fs.copyFile(path.join(output, 'audiorouter.ico'), path.resolve(__dirname, '../assets/icons/audiorouter-icon.ico'));
  console.log(output);
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
