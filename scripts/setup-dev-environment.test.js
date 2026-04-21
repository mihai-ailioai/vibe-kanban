const test = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const net = require('node:net');

const {
  findFreePort,
  DEFAULT_FRONTEND_PORT,
  DEV_PORTS_CONFIG_VERSION,
  isCurrentDevPortsConfig,
} = require('./setup-dev-environment.js');

test('findFreePort defaults to the 9000 local dev range', async () => {
  const originalCreateConnection = net.createConnection;

  net.createConnection = () => {
    const socket = new EventEmitter();
    socket.destroy = () => {};
    process.nextTick(() => {
      socket.emit('error', new Error('connect ECONNREFUSED'));
    });
    return socket;
  };

  try {
    assert.equal(DEFAULT_FRONTEND_PORT, 9000);
    assert.equal(await findFreePort(), 9000);
  } finally {
    net.createConnection = originalCreateConnection;
  }
});

test('legacy saved port allocations are not considered current', () => {
  assert.equal(
    isCurrentDevPortsConfig({
      frontend: 3000,
      backend: 3001,
      preview_proxy: 3002,
      timestamp: '2026-04-18T00:00:00.000Z',
    }),
    false,
  );

  assert.equal(
    isCurrentDevPortsConfig({
      frontend: 9000,
      backend: 9001,
      preview_proxy: 9002,
      timestamp: '2026-04-18T00:00:00.000Z',
      config_version: DEV_PORTS_CONFIG_VERSION,
    }),
    true,
  );
});
