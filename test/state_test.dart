import 'package:flutter_test/flutter_test.dart';
import 'package:localconnect/providers.dart';
import 'package:localconnect/src/rust/engine.dart';

void main() {
  test('discovery reset replaces stale devices before snapshot replay', () {
    final controller = LocalController();
    controller.event(
      const Event(
        kind: 'device',
        payload: '{"service":"stale","addresses":[]}',
      ),
    );
    controller.event(const Event(kind: 'discovery_reset', payload: '{}'));
    controller.event(
      const Event(
        kind: 'device',
        payload: '{"service":"current","addresses":["192.168.1.2:53317"]}',
      ),
    );
    expect(controller.state.devices.keys, ['current']);
    controller.dispose();
  });
  test(
    'completion preserves transfer identity and receiver export destination',
    () {
      final controller = LocalController();
      controller.event(
        const Event(
          kind: 'progress',
          payload:
              '{"id":"t1","direction":"receive","name":"/private/received/file.bin","verifiedBytes":4,"totalBytes":8,"status":"receiving"}',
        ),
      );
      controller.event(
        const Event(
          kind: 'progress',
          payload:
              '{"id":"t1","verifiedBytes":8,"totalBytes":8,"status":"complete"}',
        ),
      );
      final transfer = controller.state.transfers['t1']!;
      expect(transfer['direction'], 'receive');
      expect(transfer['name'], '/private/received/file.bin');
      expect(transfer['verifiedBytes'], 8);
      expect(transfer['status'], 'complete');
      controller.dispose();
    },
  );

  test('device address refresh and removal update the same service', () {
    final controller = LocalController();
    controller.event(
      const Event(
        kind: 'device',
        payload:
            '{"service":"peer._localconnect._tcp.local.","name":"Peer","addresses":["192.168.1.2:53317"]}',
      ),
    );
    controller.event(
      const Event(
        kind: 'device',
        payload:
            '{"service":"peer._localconnect._tcp.local.","name":"Peer","addresses":["192.168.1.3:53317"]}',
      ),
    );
    expect(controller.state.devices.length, 1);
    expect(controller.state.devices.values.single['addresses'], [
      '192.168.1.3:53317',
    ]);
    controller.event(
      const Event(
        kind: 'device_removed',
        payload: '{"service":"peer._localconnect._tcp.local."}',
      ),
    );
    expect(controller.state.devices, isEmpty);
    controller.dispose();
  });
}
