import 'dart:convert';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'src/rust/api.dart' as core;
import 'src/rust/engine.dart';

class LocalState {
  final bool ready;
  final String fingerprint;
  final int port;
  final Map<String, Map<String, dynamic>> devices;
  final Map<String, Map<String, dynamic>> transfers;
  final Map<String, Map<String, dynamic>> remoteSessions;
  final Map<String, Map<String, dynamic>> remoteFrames;
  final Map<String, String> connections;
  final String? error;
  final String? errorAction;
  const LocalState({
    this.ready = false,
    this.fingerprint = '',
    this.port = 0,
    this.devices = const {},
    this.transfers = const {},
    this.remoteSessions = const {},
    this.remoteFrames = const {},
    this.connections = const {},
    this.error,
    this.errorAction,
  });
  LocalState copy({
    bool? ready,
    String? fingerprint,
    int? port,
    Map<String, Map<String, dynamic>>? devices,
    Map<String, Map<String, dynamic>>? transfers,
    Map<String, Map<String, dynamic>>? remoteSessions,
    Map<String, Map<String, dynamic>>? remoteFrames,
    Map<String, String>? connections,
    String? error,
    String? errorAction,
  }) => LocalState(
    ready: ready ?? this.ready,
    fingerprint: fingerprint ?? this.fingerprint,
    port: port ?? this.port,
    devices: devices ?? this.devices,
    transfers: transfers ?? this.transfers,
    remoteSessions: remoteSessions ?? this.remoteSessions,
    remoteFrames: remoteFrames ?? this.remoteFrames,
    connections: connections ?? this.connections,
    error: error,
    errorAction: errorAction,
  );
}

final localProvider = StateNotifierProvider<LocalController, LocalState>(
  (ref) => LocalController(),
);

class LocalController extends StateNotifier<LocalState> {
  LocalController() : super(const LocalState());
  void started(String raw) {
    final info = jsonDecode(raw) as Map<String, dynamic>;
    state = state.copy(
      ready: true,
      fingerprint: info['fingerprint'] as String,
      port: info['port'] as int,
    );
  }

  void fail(Object error) => state = state.copy(error: error.toString());
  void stopped() => state = const LocalState();
  void event(Event event) {
    final p = jsonDecode(event.payload) as Map<String, dynamic>;
    switch (event.kind) {
      case 'connection':
        state = state.copy(
          connections: {...state.connections, '${p['peer']}': '${p['state']}'},
        );
      case 'remote_reset':
        state = state.copy(remoteSessions: {}, remoteFrames: {});
      case 'remote_active':
        final sessions = {
          ...state.remoteSessions,
          p['id'] as String: {'status': 'active', ...p},
        };
        if (sessions.length > 16) {
          final ended =
              sessions.entries
                  .where((e) => e.value['status'] == 'ended')
                  .map((e) => e.key)
                  .toList();
          if (ended.isNotEmpty) sessions.remove(ended.first);
        }
        state = state.copy(remoteSessions: sessions);
      case 'remote_frame':
        state = state.copy(
          remoteFrames: {...state.remoteFrames, p['id'] as String: p},
        );
      case 'remote_ended':
        final frames = {...state.remoteFrames}..remove(p['id']);
        state = state.copy(
          remoteFrames: frames,
          remoteSessions: {
            ...state.remoteSessions,
            p['id'] as String: {'status': 'ended'},
          },
        );
      case 'discovery_reset':
        state = state.copy(devices: {});
      case 'device':
        state = state.copy(
          devices: {...state.devices, p['service'] as String: p},
        );
      case 'device_removed':
        final devices = {...state.devices}..remove(p['service']);
        state = state.copy(devices: devices);
      case 'progress':
        final id = p['id'] as String;
        state = state.copy(
          transfers: {
            ...state.transfers,
            id: {...?state.transfers[id], ...p},
          },
        );
      case 'error':
        state = state.copy(
          error: '${p['message'] ?? p}',
          errorAction: p['action'] as String?,
        );
      default:
        break;
    }
  }

  Future<void> restoreHistory() async {
    final rows = await core.transferSnapshot();
    state = state.copy(
      transfers: {
        for (final row in rows)
          row.id: {
            'id': row.id,
            'direction': row.direction,
            'name': row.name,
            'peer': row.peer,
            'status': row.status,
            'verifiedBytes': row.verifiedBytes.toInt(),
            'totalBytes': row.totalBytes.toInt(),
            'error': row.error?.message,
            'action': row.error?.action,
          },
      },
    );
  }
}
