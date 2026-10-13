import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:file_picker/file_picker.dart';
import 'package:path_provider/path_provider.dart';
import 'providers.dart';
import 'remote_page.dart';
import 'src/rust/frb_generated.dart';
import 'src/rust/api.dart' as core;
import 'src/rust/engine.dart';
import 'src/rust/remote.dart';

const native = MethodChannel('localconnect/platform');
Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(const ProviderScope(child: MaterialApp(home: Home())));
}

class Home extends ConsumerStatefulWidget {
  const Home({super.key});
  @override
  ConsumerState<Home> createState() => _HomeState();
}

class _HomeState extends ConsumerState<Home> with WidgetsBindingObserver {
  final name = TextEditingController(text: Platform.localHostname);
  final address = TextEditingController();
  StreamSubscription<Event>? subscription;
  Peer? selected;
  String receiveDir = '';
  bool busy = false;
  Future<void>? stopping;
  Future<void>? starting;
  int operationEpoch = 0;
  final Set<String> pairingDialogs = {};
  String? projectionTicket;
  String? controlledSession;
  bool publishingFrame = false;
  int? engineGeneration;
  Geometry geometry(Map<String, dynamic> data) => Geometry(
    width: data['width'] as int,
    height: data['height'] as int,
    rotation: data['rotation'] as int,
    scale: (data['scale'] as num).toDouble(),
  );
  Future<void> nativeCapabilities() async {
    final caps = await native.invokeMapMethod<String, dynamic>('capabilities');
    final display = await native.invokeMapMethod<String, dynamic>(
      'displayGeometry',
    );
    if (display != null) {
      await core.setNativePlatform(
        geometry: geometry(display),
        input: caps?['accessibilityInput'] == true,
      );
    }
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    native.setMethodCallHandler((call) async {
      if (call.method == 'stop' &&
          (call.arguments == engineGeneration ||
              (call.arguments == 0 &&
                  busy &&
                  !ref.read(localProvider).ready))) {
        await stop();
      }
      if (call.method == 'projectionFrame') {
        final data = Map<String, dynamic>.from(call.arguments as Map);
        final id = controlledSession;
        if (id == null ||
            data['ticket'] != projectionTicket ||
            publishingFrame) {
          return;
        }
        publishingFrame = true;
        try {
          await core.publishNativeFrame(
            id: id,
            path: data['path'] as String,
            geometry: geometry(
              Map<String, dynamic>.from(data['geometry'] as Map),
            ),
            width: data['width'] as int,
            height: data['height'] as int,
          );
        } catch (error) {
          if (mounted && controlledSession == id) {
            ref.read(localProvider.notifier).fail(error);
            await core.stopRemote(id: id);
          }
        } finally {
          publishingFrame = false;
        }
      }
      if (call.method == 'projectionStopped' &&
          call.arguments == projectionTicket) {
        final id = controlledSession;
        controlledSession = null;
        projectionTicket = null;
        if (id != null) await core.stopRemote(id: id);
      }
      if (call.method == 'captureFailed' || call.method == 'inputFailed') {
        final id = controlledSession;
        if (id != null) await core.stopRemote(id: id);
        if (mounted) {
          ref
              .read(localProvider.notifier)
              .fail(call.arguments ?? 'Native capture/input stopped');
        }
      }
    });
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (Platform.isAndroid &&
        state == AppLifecycleState.resumed &&
        ref.read(localProvider).ready) {
      unawaited(
        nativeCapabilities().catchError((Object error) {
          if (mounted) ref.read(localProvider.notifier).fail(error);
        }),
      );
    }
  }

  Future<T?> run<T>(Future<T> Function() work) async {
    if (busy) return null;
    final epoch = operationEpoch;
    setState(() => busy = true);
    try {
      return await work();
    } catch (error) {
      if (mounted && epoch == operationEpoch) {
        ref.read(localProvider.notifier).fail(error);
      }
      return null;
    } finally {
      if (mounted && epoch == operationEpoch) setState(() => busy = false);
    }
  }

  Future<void> start() =>
      starting ??= startOperation().whenComplete(() => starting = null);

  Future<void> startOperation() async {
    final epoch = operationEpoch;
    engineGeneration = null;
    final dir = await getApplicationSupportDirectory();
    if (Platform.isAndroid) {
      receiveDir = '${dir.path}/received';
      final result = await native.invokeMapMethod<String, dynamic>(
        'startTransferService',
      );
      if (result?['ok'] != true) {
        throw StateError(
          result?['error'] ??
              'Allow notifications and start from the foreground.',
        );
      }
    } else {
      receiveDir =
          await FilePicker.platform.getDirectoryPath(
            dialogTitle: 'Choose a dedicated receive folder',
          ) ??
          '';
      if (receiveDir.isEmpty) return;
    }
    if (!mounted || epoch != operationEpoch) {
      if (Platform.isAndroid) {
        if (stopping != null) {
          await stopping;
        } else {
          await native.invokeMethod<void>('stopTransferService');
        }
      }
      return;
    }
    try {
      if (Platform.isAndroid) {
        await native.invokeMethod<void>('prepareIdentity');
      }
      if (!mounted || epoch != operationEpoch) return;
      final info = await core.initialize(
        dataDir: '${dir.path}/identity',
        receiveDir: receiveDir,
        name: name.text,
        port: 53317,
      );
      if (!mounted || epoch != operationEpoch) {
        await core.shutdown();
        if (Platform.isAndroid) {
          if (stopping != null) {
            await stopping;
          } else {
            await native.invokeMethod<void>('stopTransferService');
          }
        }
        return;
      }
      engineGeneration =
          (jsonDecode(info) as Map<String, dynamic>)['generation'] as int;
      if (Platform.isAndroid) {
        await native.invokeMethod<void>('bindEngine', engineGeneration);
        await native.invokeMethod<void>('sealIdentity');
      }
      ref.read(localProvider.notifier).started(info);
      if (Platform.isAndroid) await nativeCapabilities();
      if (!mounted || epoch != operationEpoch) return;
      subscription = core.events().listen((event) {
        ref.read(localProvider.notifier).event(event);
        if (event.kind == 'pair_request') {
          unawaited(
            approveIncoming(jsonDecode(event.payload) as Map<String, dynamic>),
          );
        }
        if (event.kind == 'remote_request') {
          unawaited(
            approveRemote(jsonDecode(event.payload) as Map<String, dynamic>),
          );
        }
        if (Platform.isAndroid && event.kind == 'platform_input') {
          final input = jsonDecode(event.payload) as Map<String, dynamic>;
          if (input['id'] == controlledSession) {
            unawaited(
              native
                  .invokeMethod<void>('injectInput', {
                    'id': input['id'],
                    'event': jsonEncode(input['event']),
                    'geometry': input['geometry'],
                  })
                  .catchError((Object error) async {
                    ref.read(localProvider.notifier).fail(error);
                    await core.stopRemote(id: input['id'] as String);
                  }),
            );
          }
        }
        if (Platform.isAndroid && event.kind == 'remote_active') {
          final active = jsonDecode(event.payload) as Map<String, dynamic>;
          if (active['role'] == 'controlled') {
            controlledSession = active['id'] as String;
            unawaited(
              native.invokeMethod<void>(
                'authorizeInput',
                active['input'] == true ? controlledSession : null,
              ),
            );
          }
        }
        if (Platform.isAndroid && event.kind == 'remote_ended') {
          final end = jsonDecode(event.payload) as Map<String, dynamic>;
          if (end['id'] == controlledSession) {
            controlledSession = null;
            projectionTicket = null;
            unawaited(native.invokeMethod<void>('stopProjection'));
          }
        }
        if (event.kind == 'resync') {
          unawaited(ref.read(localProvider.notifier).restoreHistory());
        }
        if (Platform.isAndroid &&
            (event.kind == 'progress' ||
                event.kind == 'error' ||
                event.kind == 'remote_frame' ||
                event.kind == 'remote_active' ||
                event.kind == 'remote_ended')) {
          final active = ref
              .read(localProvider)
              .transfers
              .values
              .any(
                (t) => [
                  'sending',
                  'receiving',
                  'reconnecting',
                ].contains(t['status']),
              );
          final remoteActive = ref
              .read(localProvider)
              .remoteSessions
              .values
              .any((session) => session['status'] == 'active');
          unawaited(
            native.invokeMethod<void>('setBusy', active || remoteActive),
          );
        }
      }, onError: (Object e) => ref.read(localProvider.notifier).fail(e));
      await ref.read(localProvider.notifier).restoreHistory();
    } catch (_) {
      if (epoch != operationEpoch) return;
      await core.shutdown();
      if (Platform.isAndroid) {
        await native.invokeMethod<void>('stopTransferService');
      }
      rethrow;
    }
  }

  Future<bool> confirm(
    String title,
    String message, {
    String approveLabel = 'Fingerprint matches',
  }) async =>
      await showDialog<bool>(
        context: context,
        barrierDismissible: false,
        builder:
            (ctx) => AlertDialog(
              title: Text(title),
              content: SingleChildScrollView(child: SelectableText(message)),
              actions: [
                TextButton(
                  onPressed: () => Navigator.pop(ctx, false),
                  child: const Text('Deny'),
                ),
                FilledButton(
                  onPressed: () => Navigator.pop(ctx, true),
                  child: Text(approveLabel),
                ),
              ],
            ),
      ) ??
      false;
  Future<void> editLimits() async {
    final epoch = operationEpoch;
    final current = await core.transferLimits();
    if (!mounted || epoch != operationEpoch) return;
    final file = TextEditingController(
      text: (current.maxFileBytes >> 30).toString(),
    );
    final total = TextEditingController(
      text: (current.maxReservedBytes >> 30).toString(),
    );
    var window = current.inFlight;
    String? validation;
    try {
      final limits = await showDialog<TransferLimits>(
        context: context,
        builder:
            (ctx) => StatefulBuilder(
              builder:
                  (ctx, update) => AlertDialog(
                    title: const Text('Transfer limits'),
                    content: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        DropdownButton<int>(
                          value: window,
                          items: [
                            for (var n = 1; n <= 8; n++)
                              DropdownMenuItem(
                                value: n,
                                child: Text('$n chunks in flight'),
                              ),
                          ],
                          onChanged: (value) => update(() => window = value!),
                        ),
                        TextField(
                          controller: file,
                          keyboardType: TextInputType.number,
                          decoration: const InputDecoration(
                            labelText: 'Maximum received file (GiB)',
                          ),
                        ),
                        TextField(
                          controller: total,
                          keyboardType: TextInputType.number,
                          decoration: const InputDecoration(
                            labelText: 'Incomplete receive quota (GiB)',
                          ),
                        ),
                        const Text(
                          'Limits apply to this device. They reserve logical file sizes, not filesystem free space.',
                        ),
                        if (validation != null)
                          Text(
                            validation!,
                            style: const TextStyle(color: Colors.red),
                          ),
                      ],
                    ),
                    actions: [
                      TextButton(
                        onPressed: () => Navigator.pop(ctx),
                        child: const Text('Cancel'),
                      ),
                      FilledButton(
                        onPressed: () {
                          final f = BigInt.tryParse(file.text);
                          final t = BigInt.tryParse(total.text);
                          final max = (BigInt.one << 64) - BigInt.one;
                          if (f == null ||
                              t == null ||
                              f <= BigInt.zero ||
                              t < f ||
                              (t << 30) > max) {
                            update(
                              () =>
                                  validation =
                                      'Enter positive whole GiB values; total must cover one file and fit u64.',
                            );
                            return;
                          }
                          Navigator.pop(
                            ctx,
                            TransferLimits(
                              inFlight: window,
                              maxFileBytes: f << 30,
                              maxReservedBytes: t << 30,
                            ),
                          );
                        },
                        child: const Text('Apply'),
                      ),
                    ],
                  ),
            ),
      );
      if (limits != null && mounted && epoch == operationEpoch) {
        await core.configureTransfer(limits: limits);
      }
    } finally {
      file.dispose();
      total.dispose();
    }
  }

  Future<void> cleanup(String id) async {
    final epoch = operationEpoch;
    final yes = await confirm(
      'Remove transfer recovery data?',
      'This removes the partial receive file and this history entry. Original source files and completed received files are retained. A removed partial cannot resume.',
      approveLabel: 'Remove recovery data',
    );
    if (!yes || !mounted || epoch != operationEpoch) return;
    await core.cleanupTransfer(id: id);
    await ref.read(localProvider.notifier).restoreHistory();
  }

  Future<void> approveIncoming(Map<String, dynamic> request) async {
    final epoch = operationEpoch;
    final fp = request['fingerprint'] as String;
    if (!pairingDialogs.add(fp)) return;
    try {
      final yes = await confirm(
        'Approve ${request['name']}?',
        'Compare this full SHA-256 fingerprint with the LOCAL fingerprint shown on the requesting device, in person or over an independent trusted channel.\n\n$fp\n\nThis grants file transfer only.',
      );
      if (!mounted || epoch != operationEpoch) return;
      await core.approvePair(fingerprint: fp, allow: yes);
    } catch (e) {
      if (mounted && epoch == operationEpoch) {
        ref.read(localProvider.notifier).fail(e);
      }
    } finally {
      pairingDialogs.remove(fp);
    }
  }

  Future<void> approveRemote(Map<String, dynamic> request) async {
    final epoch = operationEpoch;
    final id = request['request'] as String;
    if (!pairingDialogs.add(id)) return;
    try {
      final yes = await confirm(
        'Remote session request',
        'Paired peer:\n${request['peer']}\n\n${request['input'] == true ? 'Allow this device to view your primary display and inject keyboard/mouse input?' : 'Allow this device to view your primary display?'}\n\nAn active-session indicator and Stop action remain available. Approval expires after one hour; disconnect requires a new approval.',
        approveLabel:
            request['input'] == true ? 'Allow control' : 'Allow screen view',
      );
      if (!mounted || epoch != operationEpoch) return;
      if (yes && Platform.isAndroid) {
        final result = await native.invokeMapMethod<String, dynamic>(
          'startProjection',
          id,
        );
        if (result?['ok'] != true) {
          await core.approveRemote(request: id, allow: false);
          throw StateError(result?['error'] ?? 'Capture consent denied');
        }
        if (!mounted || epoch != operationEpoch) {
          await native.invokeMethod<void>('stopProjection');
          return;
        }
        projectionTicket = id;
        await nativeCapabilities();
      }
      await core.approveRemote(request: id, allow: yes);
    } catch (error) {
      if (Platform.isAndroid && projectionTicket == id) {
        projectionTicket = null;
        await native.invokeMethod<void>('stopProjection');
      }
      if (mounted && epoch == operationEpoch) {
        ref.read(localProvider.notifier).fail(error);
      }
    } finally {
      pairingDialogs.remove(id);
    }
  }

  Future<void> viewRemote(bool input) async {
    final peer = selected;
    if (peer == null) return;
    final epoch = operationEpoch;
    final session = await core.requestRemote(peer: peer, input: input);
    if (!mounted || epoch != operationEpoch) {
      await core.stopRemote(id: session.id);
      return;
    }
    await Navigator.push<void>(
      context,
      MaterialPageRoute(builder: (_) => RemotePage(session: session)),
    );
  }

  Future<void> connect(String addr) async {
    final epoch = operationEpoch;
    final peer = await core.probe(address: addr);
    if (!mounted || epoch != operationEpoch) return;
    final trusted = jsonDecode(await core.trustedPeers()) as List<dynamic>;
    if (!mounted || epoch != operationEpoch) return;
    if (trusted.any((row) => row[0] == peer.fingerprint)) {
      await core.connect(peer: peer);
      if (!mounted || epoch != operationEpoch) return;
      selected = peer;
    } else {
      final yes = await confirm(
        'Pair with ${peer.name}?',
        'Compare this full fingerprint with the LOCAL fingerprint shown on that device. Discovery names are untrusted.\n\n${peer.fingerprint}\n\nThen approve the requesting fingerprint on that device.',
      );
      if (!yes || !mounted || epoch != operationEpoch) return;
      final paired = await core.pair(
        address: addr,
        fingerprint: peer.fingerprint,
      );
      if (!mounted || epoch != operationEpoch) return;
      selected = paired;
    }
    if (mounted) setState(() {});
  }

  Future<void> send() async {
    final peer = selected;
    final epoch = operationEpoch;
    if (peer == null) throw StateError('Connect to a paired device first.');
    String? path;
    if (Platform.isAndroid) {
      // Native SAF reader streams into app-private persistent staging. No bytes cross FRB.
      path = await native.invokeMethod<String>('importFile');
    } else {
      final files = await FilePicker.platform.pickFiles(
        withData: false,
        allowMultiple: false,
      );
      path = files?.files.single.path;
    }
    if (path == null || !mounted || epoch != operationEpoch) return;
    await core.sendFile(path: path, peer: peer);
  }

  // Stop bypasses the UI busy guard and deduplicates concurrent stop requests.
  Future<void> stop() =>
      stopping ??= stopOperation().whenComplete(() => stopping = null);

  Future<void> stopOperation() async {
    operationEpoch++;
    await core.shutdown();
    await subscription?.cancel();
    subscription = null;
    if (Platform.isAndroid) {
      controlledSession = null;
      projectionTicket = null;
      await native.invokeMethod<void>('cancelStorage');
      await native.invokeMethod<void>('stopProjection');
      await native.invokeMethod<void>('stopTransferService');
    }
    selected = null;
    if (mounted) {
      Navigator.of(context).popUntil((route) => route.isFirst);
      ref.read(localProvider.notifier).stopped();
      setState(() => busy = false);
    }
  }

  Future<void> managePeers() async {
    final peers = jsonDecode(await core.trustedPeers()) as List<dynamic>;
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      builder:
          (ctx) => AlertDialog(
            title: const Text('Trusted devices'),
            content: SizedBox(
              width: 500,
              child: ListView(
                shrinkWrap: true,
                children: [
                  for (final row in peers)
                    ListTile(
                      title: Text(row[1] as String),
                      subtitle: Text(row[0] as String),
                      trailing: IconButton(
                        icon: const Icon(Icons.delete_outline),
                        onPressed: () async {
                          await core.forgetPeer(fingerprint: row[0] as String);
                          if (selected?.fingerprint == row[0]) selected = null;
                          if (ctx.mounted) Navigator.pop(ctx);
                          if (mounted) setState(() {});
                        },
                      ),
                    ),
                ],
              ),
            ),
          ),
    );
  }

  @override
  void dispose() {
    native.setMethodCallHandler(null);
    unawaited(subscription?.cancel());
    WidgetsBinding.instance.removeObserver(this);
    name.dispose();
    address.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final state = ref.watch(localProvider);
    return Scaffold(
      appBar: AppBar(
        title: const Text('LocalConnect'),
        actions: [
          if (state.ready)
            IconButton(
              onPressed: busy ? null : () => run(editLimits),
              tooltip: 'Transfer limits',
              icon: const Icon(Icons.settings),
            ),
          if (state.ready)
            TextButton(
              onPressed: () async {
                try {
                  await stop();
                } catch (error) {
                  if (mounted) ref.read(localProvider.notifier).fail(error);
                }
              },
              child: const Text('Stop all'),
            ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.all(20),
        children: [
          const Text(
            'Secure LAN transfer and explicitly approved remote sessions. Capture/input require a supported platform adapter.',
          ),
          if (state.error != null)
            Card(
              color: Colors.red.shade50,
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: SelectableText(
                  '${state.error!}\n${state.errorAction ?? ''}',
                ),
              ),
            ),
          if (!state.ready) ...[
            TextField(
              controller: name,
              decoration: const InputDecoration(
                labelText: 'Device display name',
              ),
            ),
            FilledButton(
              onPressed: busy ? null : () => run(start),
              child: const Text('Start and choose receive folder'),
            ),
          ] else ...[
            for (final session in state.remoteSessions.entries.where(
              (entry) => entry.value['status'] == 'active',
            ))
              Card(
                color: Colors.amber.shade100,
                child: ListTile(
                  title: Text('Active ${session.value['role']} session'),
                  subtitle: Text('${session.value['peer']}'),
                  trailing: FilledButton(
                    onPressed: () async {
                      try {
                        await core.stopRemote(id: session.key);
                      } catch (error) {
                        if (mounted) {
                          ref.read(localProvider.notifier).fail(error);
                        }
                      }
                    },
                    child: const Text('Stop session'),
                  ),
                ),
              ),
            SelectableText(
              'Local fingerprint (compare on other device):\n${state.fingerprint}\nListening port: ${state.port}\nReceive folder: $receiveDir',
            ),
            const SizedBox(height: 12),
            TextField(
              controller: address,
              decoration: const InputDecoration(
                labelText: 'Manual numeric LAN IP:port',
                hintText: '192.168.1.20:53317',
              ),
            ),
            Wrap(
              spacing: 8,
              children: [
                FilledButton(
                  onPressed:
                      busy
                          ? null
                          : () => run(() => connect(address.text.trim())),
                  child: const Text('Connect / pair'),
                ),
                OutlinedButton(
                  onPressed: busy ? null : () => run(managePeers),
                  child: const Text('Trusted devices'),
                ),
              ],
            ),
            for (final device in state.devices.values)
              Card(
                child: ListTile(
                  title: Text(device['name'] as String),
                  subtitle: Text((device['addresses'] as List).join('\n')),
                  trailing: TextButton(
                    onPressed:
                        busy || (device['addresses'] as List).isEmpty
                            ? null
                            : () => run(
                              () => connect(
                                (device['addresses'] as List).first as String,
                              ),
                            ),
                    child: const Text('Connect'),
                  ),
                ),
              ),
            if (selected != null)
              Text(
                'Selected: ${selected!.name} · ${selected!.address}\nConnection: ${state.connections[selected!.fingerprint] ?? 'disconnected'}',
              ),
            if (selected != null)
              Wrap(
                spacing: 8,
                children: [
                  OutlinedButton(
                    onPressed: busy ? null : () => run(() => viewRemote(false)),
                    child: const Text('Request screen view'),
                  ),
                  OutlinedButton(
                    onPressed: busy ? null : () => run(() => viewRemote(true)),
                    child: const Text('Request remote control'),
                  ),
                ],
              ),
            FilledButton(
              onPressed: busy || selected == null ? null : () => run(send),
              child: const Text('Select and send file'),
            ),
            if (Platform.isAndroid)
              OutlinedButton(
                onPressed: () => native.invokeMethod<void>('batterySettings'),
                child: const Text('Battery settings guidance'),
              ),
            if (Platform.isAndroid)
              OutlinedButton(
                onPressed:
                    () => native.invokeMethod<void>('accessibilitySettings'),
                child: const Text('Enable Accessibility input manually'),
              ),
            for (final entry in state.transfers.entries)
              Card(
                child: Padding(
                  padding: const EdgeInsets.all(12),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      SelectableText(
                        '${entry.value['name'] ?? entry.key}\n${entry.value['status']}',
                      ),
                      if (entry.value['totalBytes'] is num)
                        LinearProgressIndicator(
                          value:
                              (entry.value['totalBytes'] as num) == 0
                                  ? 1
                                  : ((entry.value['verifiedBytes'] ?? 0)
                                          as num) /
                                      (entry.value['totalBytes'] as num),
                        ),
                      Text(
                        '${entry.value['verifiedBytes'] ?? 0} verified bytes · ${((entry.value['bytesPerSecond'] ?? 0) as num).toStringAsFixed(0)} B/s',
                      ),
                      if (entry.value['error'] != null)
                        Text('${entry.value['error']}'),
                      Wrap(
                        spacing: 8,
                        children: [
                          TextButton(
                            onPressed: () => run(() => cleanup(entry.key)),
                            child: const Text('Cleanup'),
                          ),
                          TextButton(
                            onPressed:
                                () => run(
                                  () => core.cancelTransfer(id: entry.key),
                                ),
                            child: const Text('Cancel'),
                          ),
                          if (entry.value['direction'] != 'receive')
                            TextButton(
                              onPressed:
                                  () => run(() async {
                                    if (address.text.trim().isNotEmpty) {
                                      await core.updateTransferAddress(
                                        id: entry.key,
                                        address: address.text.trim(),
                                      );
                                    }
                                    await core.resumeTransfer(id: entry.key);
                                  }),
                              child: const Text('Resume'),
                            ),
                          if (Platform.isAndroid &&
                              entry.value['status'] == 'complete' &&
                              entry.value['name'] is String &&
                              entry.value['direction'] == 'receive')
                            TextButton(
                              onPressed:
                                  () => run(
                                    () => native.invokeMethod<void>(
                                      'exportFile',
                                      entry.value['name'],
                                    ),
                                  ),
                              child: const Text('Export with SAF'),
                            ),
                        ],
                      ),
                    ],
                  ),
                ),
              ),
          ],
          if (busy) const LinearProgressIndicator(),
        ],
      ),
    );
  }
}
