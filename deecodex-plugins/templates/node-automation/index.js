const { createPlugin } = require('./deecodex-plugin');

let config = {};
let currentRun = null;

createPlugin({
  initialize(params) {
    config = params.config || {};
    return { ok: true, name: 'Example Node Automation' };
  },
  notifications: {
    'config.update': (params) => {
      config = params.config || {};
    },
  },
  methods: {
    'automation.run': (params) => {
      currentRun = {
        task: params.task || config.default_task || 'health-check',
        dry_run: params.dry_run !== false,
        started_at: Math.floor(Date.now() / 1000),
        state: 'running',
      };
      return { ok: true, run: currentRun };
    },
    'automation.status': () => ({ ok: true, run: currentRun || { state: 'idle' } }),
    'automation.stop': () => {
      if (currentRun) currentRun.state = 'stopped';
      return { ok: true, run: currentRun || { state: 'idle' } };
    },
  },
}).start();
