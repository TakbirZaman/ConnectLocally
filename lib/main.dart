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
  runApp(
    ProviderScope(
      child: MaterialApp(
        debugShowCheckedModeBanner: false,
        title: 'LocalConnect',
        theme: ThemeData(
          useMaterial3: true,
          colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xff155eef)),
          scaffoldBackgroundColor: const Color(0xfff5f7fb),
          inputDecorationTheme: const InputDecorationTheme(
            border: OutlineInputBorder(),
            filled: true,
            fillColor: Colors.white,
          ),
        ),
        home: const Home(),
      ),
    ),
  );
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
  int page = 0;
  Future<void>? stopping;
  Future<void>? starting;
  int operationEpoch = 0;
  final Set<String> pairingDialogs = {};
  String? projectionTicket;
  String? controlledSession;
  bool publishingFrame = false;
  bool androidInputSupported = false;
  int? engineGeneration;
  String? displayedPairCode;
  BuildContext? pairCodeContext;
  Timer? pairCodeTimer;
  Geometry geometry(Map<String, dynamic> data) => Geometry(
    width: data['width'] as int,
    height: data['height'] as int,
    rotation: data['rotation'] as int,
    scale: (data['scale'] as num).toDouble(),
  );
  Future<void> nativeCapabilities() async {
    final caps = await native.invokeMapMethod<String, dynamic>('capabilities');
    if (mounted) {
      setState(
        () => androidInputSupported = caps?['accessibilitySupported'] == true,
      );
    }
    final display = await native.invokeMapMethod<String, dynamic>(
      'displayGeometry',
    );
    if (display != null) {
      await core.setNativePlatform(
        geometry: geometry(display),
        input: androidInputSupported && caps?['accessibilityInput'] == true,
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
        if (!mounted || epoch != operationEpoch) return;
        ref.read(localProvider.notifier).event(event);
        if (event.kind == 'paired') {
          final data = jsonDecode(event.payload) as Map<String, dynamic>;
          if (data['address'] is String) {
            setState(
              () =>
                  selected = Peer(
                    fingerprint: data['fingerprint'] as String,
                    name: data['name'] as String,
                    address: data['address'] as String,
                  ),
            );
          }
          final dialog = pairCodeContext;
          if (data['codeWindow'] == displayedPairCode &&
              dialog != null &&
              dialog.mounted) {
            Navigator.of(dialog).pop();
          }
        }
        if (event.kind == 'pair_request' && displayedPairCode != null) {
          final request = jsonDecode(event.payload) as Map<String, dynamic>;
          unawaited(
            core.approvePair(
              fingerprint: request['fingerprint'] as String,
              allow: false,
            ),
          );
        } else if (event.kind == 'pair_request') {
          unawaited(
            approveIncoming(jsonDecode(event.payload) as Map<String, dynamic>),
          );
        }
        if (event.kind == 'remote_request' && displayedPairCode != null) {
          final request = jsonDecode(event.payload) as Map<String, dynamic>;
          unawaited(
            core.approveRemote(
              request: request['request'] as String,
              allow: false,
            ),
          );
        } else if (event.kind == 'remote_request') {
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
        'Compare both fingerprints with the requesting device. Names alone do not identify a device.\n\nTheir fingerprint:\n$fp\n\nYour fingerprint:\n${ref.read(localProvider).fingerprint}\n\nPairing allows file transfer. Remote access requires separate approval.',
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
      final peers = jsonDecode(await core.trustedPeers()) as List<dynamic>;
      if (!mounted || epoch != operationEpoch) return;
      final names = peers.where((row) => row[0] == request['peer']).toList();
      final peerName =
          names.isEmpty ? 'Paired device' : names.first[1] as String;
      final yes = await confirm(
        'Remote session request',
        'Paired device: $peerName\n\n${request['input'] == true ? (Platform.isAndroid ? 'Allow this device to view your phone and use supported taps, swipes and text actions?' : 'Allow this device to view your display and use keyboard/mouse input?') : 'Allow this device to view your screen?'}\n\nAn active-session indicator and Stop action remain available. Approval expires after one hour; disconnect requires a new approval.',
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

  Future<void> displayCode() async {
    final epoch = operationEpoch;
    var remember = false;
    final choice = await showDialog<bool>(
      context: context,
      builder:
          (ctx) => StatefulBuilder(
            builder:
                (ctx, update) => AlertDialog(
                  title: const Text('Connect with a one-time code'),
                  content: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const Text(
                        'A code will appear on this device. Enter it on the other device to connect.',
                      ),
                      CheckboxListTile(
                        contentPadding: EdgeInsets.zero,
                        value: remember,
                        onChanged:
                            (value) => update(() => remember = value ?? false),
                        title: const Text('Remember the connecting device'),
                        subtitle: const Text(
                          'Off: access ends when LocalConnect stops. Enable on both devices to reconnect later.',
                        ),
                      ),
                    ],
                  ),
                  actions: [
                    TextButton(
                      onPressed: () => Navigator.pop(ctx),
                      child: const Text('Cancel'),
                    ),
                    FilledButton(
                      onPressed: () => Navigator.pop(ctx, true),
                      child: const Text('Generate code'),
                    ),
                  ],
                ),
          ),
    );
    if (choice != true || !mounted || epoch != operationEpoch) return;
    final data =
        jsonDecode(await core.beginPairingCode(remember: remember))
            as Map<String, dynamic>;
    if (!mounted || epoch != operationEpoch) return;
    displayedPairCode = data['id'] as String;
    var remaining = (data['expires_in_seconds'] as num).toInt();
    final clock = Stopwatch()..start();
    final lifetime = remaining;
    StateSetter? refresh;
    try {
      pairCodeTimer = Timer.periodic(const Duration(seconds: 1), (_) {
        remaining = lifetime - clock.elapsed.inSeconds;
        final dialog = pairCodeContext;
        if (remaining <= 0 && dialog != null && dialog.mounted) {
          Navigator.of(dialog).pop();
        } else if (dialog != null && dialog.mounted) {
          refresh?.call(() {});
        }
      });
      await showDialog<void>(
        context: context,
        builder:
            (ctx) => StatefulBuilder(
              builder: (ctx, update) {
                pairCodeContext = ctx;
                refresh = update;
                return AlertDialog(
                  title: const Text('Enter this code on the other device'),
                  content: SingleChildScrollView(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        FittedBox(
                          fit: BoxFit.scaleDown,
                          child: Text(
                            data['code'] as String,
                            maxLines: 1,
                            softWrap: false,
                            style: const TextStyle(
                              fontSize: 40,
                              letterSpacing: 6,
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                        ),
                        const SizedBox(height: 16),
                        Text(
                          'Expires in ${remaining}s · single use · 3 attempts',
                        ),
                        const SizedBox(height: 12),
                        Text(
                          remember
                              ? 'The connecting device will be remembered.'
                              : 'Session only: access ends when LocalConnect stops.',
                        ),
                        const SizedBox(height: 12),
                        const Text(
                          'On the other device, select this device and enter the code. Remote access still needs separate approval.',
                        ),
                      ],
                    ),
                  ),
                  actions: [
                    TextButton(
                      onPressed: () => stop(),
                      child: const Text('Stop all'),
                    ),
                    FilledButton(
                      onPressed: () => Navigator.pop(ctx),
                      child: const Text('Close code'),
                    ),
                  ],
                );
              },
            ),
      );
    } finally {
      pairCodeTimer?.cancel();
      pairCodeTimer = null;
      pairCodeContext = null;
      displayedPairCode = null;
      if (mounted && epoch == operationEpoch && ref.read(localProvider).ready) {
        await core.cancelPairingCode();
      }
    }
  }

  Future<Peer?> enterCode(Peer peer) async {
    final epoch = operationEpoch;
    final code = TextEditingController();
    var remember = false;
    var submitting = false;
    String? error;
    final route = DialogRoute<Peer>(
        context: context,
        barrierDismissible: false,
        builder:
            (ctx) => StatefulBuilder(
              builder: (ctx, update) {
                Future<void> submit() async {
                  if (submitting) return;
                  if (!RegExp(r'^[0-9]{6}$').hasMatch(code.text)) {
                    update(() => error = 'Enter all six digits.');
                    return;
                  }
                  update(() {
                    submitting = true;
                    error = null;
                  });
                  try {
                    final paired = await core.pairWithCode(
                      address: peer.address,
                      code: code.text,
                      remember: remember,
                    );
                    if (ctx.mounted && mounted && epoch == operationEpoch) {
                      Navigator.pop(ctx, paired);
                    }
                  } catch (failure) {
                    if (ctx.mounted) {
                      update(
                        () =>
                            error =
                                'Could not pair. Check the code, generate a fresh one if expired, and use the latest app on both devices.\n$failure',
                      );
                    }
                  } finally {
                    if (ctx.mounted) update(() => submitting = false);
                  }
                }

                return AlertDialog(
                  title: Text('Connect to ${peer.name}'),
                  content: SingleChildScrollView(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const Text(
                          'On the other device, choose Show pairing code. Enter that code here. Device names alone are not proof of identity.',
                        ),
                        const SizedBox(height: 16),
                        TextField(
                          controller: code,
                          enabled: !submitting,
                          autofocus: true,
                          keyboardType: TextInputType.number,
                          enableSuggestions: false,
                          autocorrect: false,
                          inputFormatters: [
                            FilteringTextInputFormatter.digitsOnly,
                          ],
                          maxLength: 6,
                          decoration: const InputDecoration(
                            labelText: 'One-time code',
                            hintText: '123456',
                          ),
                          onSubmitted: (_) => submit(),
                        ),
                        CheckboxListTile(
                          contentPadding: EdgeInsets.zero,
                          value: remember,
                          onChanged:
                              submitting
                                  ? null
                                  : (value) =>
                                      update(() => remember = value ?? false),
                          title: Text('Remember ${peer.name}'),
                          subtitle: const Text(
                            'Off: session only. Enable on both devices to reconnect later.',
                          ),
                        ),
                        if (error != null)
                          Text(
                            error!,
                            style: TextStyle(
                              color: Theme.of(ctx).colorScheme.error,
                            ),
                          ),
                        if (submitting) const LinearProgressIndicator(),
                      ],
                    ),
                  ),
                  actions: [
                    TextButton(
                      onPressed: () => stop(),
                      child: const Text('Stop all'),
                    ),
                    TextButton(
                      onPressed: submitting ? null : () => Navigator.pop(ctx),
                      child: const Text('Cancel'),
                    ),
                    FilledButton(
                      onPressed: submitting ? null : submit,
                      child: const Text('Connect'),
                    ),
                  ],
                );
              },
            ),
      );
    try {
      return await Navigator.of(context).push<Peer>(route);
    } finally {
      await route.completed;
      code.clear();
      code.dispose();
    }
  }

  Future<void> connect(String addr, {bool pairAgain = false}) async {
    final epoch = operationEpoch;
    final peer = await core.probe(address: addr);
    if (!mounted || epoch != operationEpoch) return;
    final trusted = jsonDecode(await core.trustedPeers()) as List<dynamic>;
    if (!mounted || epoch != operationEpoch) return;
    if (!pairAgain && trusted.any((row) => row[0] == peer.fingerprint)) {
      await core.connect(peer: peer);
      if (!mounted || epoch != operationEpoch) return;
      selected = peer;
    } else {
      final paired = await enterCode(peer);
      if (paired == null || !mounted || epoch != operationEpoch) return;
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
      setState(() {
        busy = false;
        page = 0;
      });
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
                          if (selected?.fingerprint == row[0]) {
                            selected = null;
                          }
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
    pairCodeTimer?.cancel();
    native.setMethodCallHandler(null);
    unawaited(subscription?.cancel());
    WidgetsBinding.instance.removeObserver(this);
    name.dispose();
    address.dispose();
    super.dispose();
  }

  Future<void> showPassport(String id) async {
    final epoch = operationEpoch;
    final history = jsonDecode(await core.transferHistory()) as List<dynamic>;
    final matches = history.where((row) => row['id'] == id).toList();
    if (matches.isEmpty || matches.first['status'] != 'complete') {
      throw StateError(
        'A passport is available only for a completed transfer.',
      );
    }
    final row = Map<String, dynamic>.from(matches.first as Map);
    final passport = <String, dynamic>{
      'format': 'localconnect-transfer-passport',
      'version': 1,
      'transferId': id,
      'fileName': row['fileName'],
      'fileBytes': row['fileBytes'],
      'sha256': row['sha256'],
      'peerFingerprint': row['peer'],
      'direction': row['direction'],
      'recordedAtUtc':
          DateTime.fromMillisecondsSinceEpoch(
            (row['updatedAt'] as num).toInt() * 1000,
            isUtc: true,
          ).toIso8601String(),
    };
    if (!mounted || epoch != operationEpoch) return;
    await showDialog<void>(
      context: context,
      builder:
          (ctx) => AlertDialog(
            title: const Text('Transfer passport'),
            content: SingleChildScrollView(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    '${row['fileName']}',
                    style: Theme.of(ctx).textTheme.titleMedium,
                  ),
                  Text(bytes(row['fileBytes'] as num)),
                  const SizedBox(height: 16),
                  const Text('SHA-256 verified at completion'),
                  const SizedBox(height: 8),
                  SelectableText('${row['sha256']}'),
                  const SizedBox(height: 16),
                  const Text('Peer fingerprint'),
                  const SizedBox(height: 8),
                  SelectableText('${row['peer']}'),
                  const SizedBox(height: 16),
                  const Text(
                    'An offline integrity record, not a signed receipt or a recheck of later file edits. Compare the hash with an independently trusted original.',
                  ),
                ],
              ),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(ctx),
                child: const Text('Close'),
              ),
              FilledButton.icon(
                onPressed: () async {
                  try {
                    await Clipboard.setData(
                      ClipboardData(
                        text: const JsonEncoder.withIndent(
                          '  ',
                        ).convert(passport),
                      ),
                    );
                    if (mounted && ctx.mounted) {
                      ScaffoldMessenger.of(context).showSnackBar(
                        const SnackBar(
                          content: Text(
                            'Passport copied. Share only with people you trust.',
                          ),
                        ),
                      );
                    }
                  } catch (error) {
                    if (mounted) ref.read(localProvider.notifier).fail(error);
                  }
                },
                icon: const Icon(Icons.copy),
                label: const Text('Copy passport'),
              ),
            ],
          ),
    );
  }

  String bytes(num value) {
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    var amount = value.toDouble();
    var unit = 0;
    while (amount >= 1024 && unit < units.length - 1) {
      amount /= 1024;
      unit++;
    }
    return '${amount.toStringAsFixed(unit == 0 ? 0 : 1)} ${units[unit]}';
  }

  Widget panel(
    String title,
    String description,
    List<Widget> children, {
    IconData? icon,
  }) => Card(
    elevation: 0,
    margin: const EdgeInsets.only(bottom: 16),
    child: Padding(
      padding: const EdgeInsets.all(20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (icon != null)
            Align(
              alignment: Alignment.centerLeft,
              child: Icon(
                icon,
                size: 30,
                color: Theme.of(context).colorScheme.primary,
              ),
            ),
          const SizedBox(height: 8),
          Text(title, style: Theme.of(context).textTheme.titleLarge),
          const SizedBox(height: 6),
          Text(description, style: Theme.of(context).textTheme.bodyMedium),
          const SizedBox(height: 16),
          ...children,
        ],
      ),
    ),
  );

  Widget transferCard(String id, Map<String, dynamic> transfer) {
    final status = '${transfer['status'] ?? 'pending'}';
    final active = [
      'sending',
      'receiving',
      'reconnecting',
      'hashing',
    ].contains(status);
    final receiving = transfer['direction'] == 'receive';
    final total = (transfer['totalBytes'] as num?) ?? 0;
    final verified = (transfer['verifiedBytes'] as num?) ?? 0;
    final speed = (transfer['bytesPerSecond'] as num?) ?? 0;
    final rawName = '${transfer['name'] ?? 'File transfer'}';
    final fileName = rawName.replaceAll('\\', '/').split('/').last;
    return panel(
      fileName,
      '${receiving ? 'Receiving' : 'Sending'} · ${status.replaceAll('_', ' ')}',
      [
        if (total > 0) ...[
          LinearProgressIndicator(
            value: (verified / total).clamp(0.0, 1.0).toDouble(),
          ),
          const SizedBox(height: 10),
        ],
        Text('${bytes(verified)} verified of ${bytes(total)}'),
        if (active) Text('${bytes(speed)}/s'),
        if (transfer['error'] != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              '${transfer['error']}',
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ),
        const SizedBox(height: 8),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            if (status == 'complete')
              OutlinedButton.icon(
                onPressed: busy ? null : () => run(() => showPassport(id)),
                icon: const Icon(Icons.verified_outlined),
                label: const Text('Integrity passport'),
              ),
            if (active)
              TextButton(
                onPressed:
                    busy ? null : () => run(() => core.cancelTransfer(id: id)),
                child: const Text('Cancel transfer'),
              ),
            if (!active && status != 'complete' && !receiving)
              FilledButton.tonal(
                onPressed:
                    busy
                        ? null
                        : () => run(() async {
                          if (address.text.trim().isNotEmpty) {
                            await core.updateTransferAddress(
                              id: id,
                              address: address.text.trim(),
                            );
                          }
                          await core.resumeTransfer(id: id);
                        }),
                child: const Text('Resume transfer'),
              ),
            if (Platform.isAndroid &&
                status == 'complete' &&
                receiving &&
                transfer['name'] is String)
              FilledButton.icon(
                onPressed:
                    busy
                        ? null
                        : () => run(
                          () => native.invokeMethod<void>(
                            'exportFile',
                            transfer['name'],
                          ),
                        ),
                icon: const Icon(Icons.save_alt),
                label: const Text('Save to phone'),
              ),
            if (!active)
              TextButton(
                onPressed: busy ? null : () => run(() => cleanup(id)),
                child: Text(
                  status == 'complete'
                      ? 'Remove from history'
                      : 'Remove partial file',
                ),
              ),
          ],
        ),
      ],
      icon:
          status == 'complete'
              ? Icons.check_circle_outline
              : receiving
              ? Icons.download_outlined
              : Icons.upload_outlined,
    );
  }

  @override
  Widget build(BuildContext context) {
    final state = ref.watch(localProvider);
    final titles = ['Devices', 'Files', 'Remote', 'Settings'];
    final sessions = state.remoteSessions.entries.where(
      (entry) => entry.value['status'] == 'active',
    );
    final content = <Widget>[];
    if (!state.ready) {
      content.add(
        panel(
          'Connect your devices',
          'Send files and share screens over your local Wi-Fi. Start on both devices to find each other.',
          [
            TextField(
              controller: name,
              decoration: const InputDecoration(
                labelText: 'This device’s name',
                prefixIcon: Icon(Icons.devices),
              ),
            ),
            const SizedBox(height: 16),
            FilledButton.icon(
              onPressed: busy ? null : () => run(start),
              icon: const Icon(Icons.wifi),
              label: Text(busy ? 'Starting…' : 'Start LocalConnect'),
            ),
            const SizedBox(height: 12),
            Text(
              Platform.isAndroid
                  ? 'Received files stay in the app until you choose Save to phone.'
                  : 'You will choose a folder for received files.',
            ),
          ],
          icon: Icons.connect_without_contact,
        ),
      );
    } else if (page == 0) {
      content.add(
        panel(
          'Connect with a code',
          'Show a one-time code here, then enter it on the other device.',
          [
            FilledButton.icon(
              onPressed: busy ? null : () => run(displayCode),
              icon: const Icon(Icons.password),
              label: const Text('Show pairing code'),
            ),
          ],
          icon: Icons.link,
        ),
      );
      if (selected != null) {
        content.add(
          panel(
            selected!.name,
            'Selected device · ${state.connections[selected!.fingerprint] ?? 'connecting'}',
            [
              Text(selected!.address),
              const SizedBox(height: 12),
              FilledButton.icon(
                onPressed: busy ? null : () => setState(() => page = 1),
                icon: const Icon(Icons.send_outlined),
                label: const Text('Send a file'),
              ),
            ],
            icon: Icons.devices,
          ),
        );
      }
      content.add(
        panel(
          'Nearby devices',
          'Start LocalConnect on another device connected to the same Wi-Fi.',
          [
            if (state.devices.isEmpty)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 16),
                child: Text(
                  'Looking for devices…\nIf none appear, use an IP address below.',
                ),
              ),
            for (final device in state.devices.values)
              ListTile(
                contentPadding: EdgeInsets.zero,
                leading: const CircleAvatar(child: Icon(Icons.devices_other)),
                title: Text('${device['name']}'),
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
          ],
        ),
      );
      content.add(
        Card(
          elevation: 0,
          child: ExpansionTile(
            title: const Text('Connect by IP address'),
            subtitle: const Text(
              'Use this when automatic discovery is unavailable',
            ),
            leading: const Icon(Icons.link),
            childrenPadding: const EdgeInsets.all(20),
            children: [
              TextField(
                controller: address,
                keyboardType: TextInputType.url,
                decoration: const InputDecoration(
                  labelText: 'Device IP and port',
                  hintText: '192.168.1.20:53317',
                ),
                onSubmitted:
                    busy
                        ? null
                        : (_) => run(() => connect(address.text.trim())),
              ),
              const SizedBox(height: 12),
              FilledButton(
                onPressed:
                    busy ? null : () => run(() => connect(address.text.trim())),
                child: const Text('Connect'),
              ),
            ],
          ),
        ),
      );
    } else if (page == 1) {
      content.add(
        panel(
          'Send a file',
          selected == null
              ? 'Connect to a device in the Devices tab first.'
              : 'Send securely to ${selected!.name}. Interrupted transfers can resume.',
          [
            FilledButton.icon(
              onPressed: busy || selected == null ? null : () => run(send),
              icon: const Icon(Icons.upload_file),
              label: const Text('Choose file to send'),
            ),
          ],
          icon: Icons.folder_outlined,
        ),
      );
      content.add(
        Padding(
          padding: const EdgeInsets.only(bottom: 12),
          child: Text(
            'Transfer history',
            style: Theme.of(context).textTheme.titleMedium,
          ),
        ),
      );
      if (state.transfers.isEmpty) {
        content.add(
          panel(
            'No transfers yet',
            'Files you send and receive will appear here with their progress.',
            [],
            icon: Icons.swap_vert,
          ),
        );
      }
      for (final entry in state.transfers.entries) {
        content.add(transferCard(entry.key, entry.value));
      }
    } else if (page == 2) {
      content.add(
        panel(
          'View or control another device',
          selected == null
              ? 'Connect to a device in the Devices tab first.'
              : 'Request access to ${selected!.name}. The other device must approve each session.',
          [
            FilledButton.icon(
              onPressed:
                  busy || selected == null
                      ? null
                      : () => run(() => viewRemote(false)),
              icon: const Icon(Icons.screen_share_outlined),
              label: const Text('Request screen view'),
            ),
            const SizedBox(height: 12),
            OutlinedButton.icon(
              onPressed:
                  busy || selected == null
                      ? null
                      : () => run(() => viewRemote(true)),
              icon: const Icon(Icons.touch_app_outlined),
              label: const Text('Request remote control'),
            ),
            const SizedBox(height: 12),
            const Text(
              'You stay in control: access requires approval and can be stopped at any time.',
            ),
          ],
          icon: Icons.desktop_windows_outlined,
        ),
      );
      if (Platform.isAndroid) {
        content.add(
          panel(
            androidInputSupported
                ? 'Allow control of this phone'
                : 'Phone input is unavailable',
            androidInputSupported
                ? 'Android requires screen-capture consent for each session. To accept gestures, enable LocalConnect in Accessibility settings yourself.'
                : 'This View Only build supports files and approved screen viewing. Accessibility-based control of this phone is not included.',
            [
              if (androidInputSupported)
                OutlinedButton.icon(
                  onPressed:
                      busy
                          ? null
                          : () => run(
                            () => native.invokeMethod<void>(
                              'accessibilitySettings',
                            ),
                          ),
                  icon: const Icon(Icons.accessibility_new),
                  label: const Text('Open Accessibility settings'),
                ),
            ],
          ),
        );
      }
    } else {
      content.add(
        panel(
          'This device',
          'Listening on port ${state.port}',
          [
            Text(name.text, style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 12),
            const Text(
              'Advanced: cryptographic device identity (no fingerprint scanner required).',
            ),
            const SizedBox(height: 8),
            SelectableText(state.fingerprint),
            TextButton.icon(
              onPressed:
                  () => run(
                    () => Clipboard.setData(
                      ClipboardData(text: state.fingerprint),
                    ),
                  ),
              icon: const Icon(Icons.copy),
              label: const Text('Copy fingerprint'),
            ),
            const Divider(),
            Text(
              Platform.isAndroid
                  ? 'Received files: private app storage'
                  : 'Receive folder: $receiveDir',
            ),
          ],
          icon: Icons.verified_user_outlined,
        ),
      );
      content.add(
        panel(
          'Preferences',
          'Manage trust, storage limits, and device settings.',
          [
            ListTile(
              contentPadding: EdgeInsets.zero,
              leading: const Icon(Icons.devices),
              title: const Text('Trusted devices'),
              subtitle: const Text('Review or remove paired devices'),
              trailing: const Icon(Icons.chevron_right),
              onTap: busy ? null : () => run(managePeers),
            ),
            ListTile(
              contentPadding: EdgeInsets.zero,
              leading: const Icon(Icons.tune),
              title: const Text('Transfer limits'),
              subtitle: const Text('Advanced storage and transfer settings'),
              trailing: const Icon(Icons.chevron_right),
              onTap: busy ? null : () => run(editLimits),
            ),
            if (Platform.isAndroid)
              ListTile(
                contentPadding: EdgeInsets.zero,
                leading: const Icon(Icons.battery_saver_outlined),
                title: const Text('Battery settings'),
                subtitle: const Text(
                  'Review background restrictions on this phone',
                ),
                trailing: const Icon(Icons.chevron_right),
                onTap:
                    busy
                        ? null
                        : () => run(
                          () => native.invokeMethod<void>('batterySettings'),
                        ),
              ),
          ],
        ),
      );
    }
    return Scaffold(
      appBar: AppBar(
        title: const Text('LocalConnect'),
        actions: [
          if (state.ready)
            TextButton.icon(
              onPressed: () async {
                try {
                  await stop();
                } catch (error) {
                  if (mounted) ref.read(localProvider.notifier).fail(error);
                }
              },
              icon: const Icon(Icons.stop_circle_outlined),
              label: const Text('Stop all'),
            ),
        ],
      ),
      bottomNavigationBar:
          state.ready
              ? NavigationBar(
                selectedIndex: page,
                onDestinationSelected: (value) => setState(() => page = value),
                destinations: const [
                  NavigationDestination(
                    icon: Icon(Icons.devices_outlined),
                    label: 'Devices',
                  ),
                  NavigationDestination(
                    icon: Icon(Icons.folder_outlined),
                    label: 'Files',
                  ),
                  NavigationDestination(
                    icon: Icon(Icons.screen_share_outlined),
                    label: 'Remote',
                  ),
                  NavigationDestination(
                    icon: Icon(Icons.settings_outlined),
                    label: 'Settings',
                  ),
                ],
              )
              : null,
      body: SafeArea(
        child: Align(
          alignment: Alignment.topCenter,
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 820),
            child: ListView(
              padding: const EdgeInsets.all(16),
              children: [
                if (busy)
                  const Padding(
                    padding: EdgeInsets.only(bottom: 12),
                    child: LinearProgressIndicator(),
                  ),
                if (state.ready)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 16),
                    child: Text(
                      titles[page],
                      style: Theme.of(context).textTheme.headlineSmall,
                    ),
                  ),
                if (state.error != null)
                  panel(
                    'Something needs attention',
                    '${state.error}\n${state.errorAction ?? ''}',
                    [],
                    icon: Icons.error_outline,
                  ),
                for (final session in sessions)
                  panel(
                    'Screen session is active',
                    '${session.value['role']} · ${session.value['peer']}',
                    [
                      FilledButton.icon(
                        onPressed: () async {
                          try {
                            await core.stopRemote(id: session.key);
                          } catch (error) {
                            if (mounted) {
                              ref.read(localProvider.notifier).fail(error);
                            }
                          }
                        },
                        icon: const Icon(Icons.stop),
                        label: const Text('Stop session'),
                      ),
                    ],
                    icon: Icons.visibility_outlined,
                  ),
                ...content,
              ],
            ),
          ),
        ),
      ),
    );
  }
}
