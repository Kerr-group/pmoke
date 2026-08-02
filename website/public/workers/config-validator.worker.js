let wasm;

self.onmessage = async (event) => {
  const message = event.data;
  if (message?.type === 'init') {
    try {
      wasm = await import(`${message.basePath ?? ''}/wasm/pmoke_web_wasm.js`);
      await wasm.default();
      self.postMessage({ type: 'ready', build: wasm.build_info() });
    } catch (error) {
      self.postMessage({ type: 'error', message: String(error) });
    }
    return;
  }

  if (message?.type === 'validate' && wasm) {
    try {
      const report = JSON.parse(wasm.validate_config_toml(message.input));
      if (
        report === null ||
        typeof report !== 'object' ||
        typeof report.valid !== 'boolean' ||
        !Array.isArray(report.diagnostics)
      ) {
        throw new Error('invalid validation report');
      }
      self.postMessage({ type: 'result', id: message.id, report });
    } catch (error) {
      self.postMessage({ type: 'error', id: message.id, message: String(error) });
    }
  }
};
