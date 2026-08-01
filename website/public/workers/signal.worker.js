let wasm;

self.onmessage = async (event) => {
  if (event.data?.type !== 'init') return;
  try {
    const basePath = event.data.basePath ?? '';
    wasm = await import(`${basePath}/wasm/pmoke_web_wasm.js`);
    await wasm.default();
    const generated = wasm.generate_signal(event.data.samples ?? 720, 0.17);
    const data = generated instanceof Float64Array ? generated : Float64Array.from(generated);
    self.postMessage({ type: 'ready', data: data.buffer, build: wasm.build_info() }, [data.buffer]);
  } catch (error) {
    self.postMessage({ type: 'error', message: String(error) });
  }
};
