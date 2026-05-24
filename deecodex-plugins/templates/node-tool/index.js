const { createPlugin } = require('./deecodex-plugin');

let config = {};

createPlugin({
  initialize(params) {
    config = params.config || {};
    return { ok: true, name: 'Example Node Tool' };
  },
  notifications: {
    'config.update': (params) => {
      config = params.config || {};
    },
  },
  methods: {
    'example.status': async (_params, host) => {
      const now = Math.floor(Date.now() / 1000);
      await host.cache.write('last-status.json', JSON.stringify({ ts: now }));
      const cached = await host.cache.read('last-status.json');
      return {
        ok: true,
        message: config.message || 'ready',
        cache: JSON.parse(cached.content || '{}'),
        ts: now,
      };
    },
  },
}).start();
