import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'providers.dart';
import 'src/rust/api.dart' as core;
import 'src/rust/remote_engine.dart';
import 'src/rust/remote.dart';

class RemotePage extends ConsumerStatefulWidget {
  final RemoteView session;
  const RemotePage({super.key, required this.session});
  @override
  ConsumerState<RemotePage> createState() => _RemotePageState();
}

class _RemotePageState extends ConsumerState<RemotePage> {
  final focus = FocusNode();
  final text = TextEditingController();
  final keys = <int>{};
  final physicalKeys = <int, int>{};
  final buttons = <int>{};
  Uint8List? image;
  int sequence = -1;
  int load = 0;
  bool loadingFrame = false;
  Map<String, dynamic>? pendingFrame;
  double aspect = 16 / 9;
  DateTime lastMove = DateTime.fromMillisecondsSinceEpoch(0);
  bool ended = false;
  Geometry? imageGeometry;
  Future<void> inputTail = Future.value();
  int queued = 0;
  Offset? swipeStart;
  Offset? swipeEnd;
  bool get android => widget.session.platform == 'android';
  Offset normalized(Offset point, Size size) => Offset(
    (point.dx / size.width).clamp(0.0, 1.0),
    (point.dy / size.height).clamp(0.0, 1.0),
  );
  Future<void> send(String type, Map<String, dynamic> data) {
    if (!widget.session.input || ended || image == null) return Future.value();
    if (queued >= 128) {
      unawaited(core.stopRemote(id: widget.session.id));
      return Future.value();
    }
    queued++;
    final display = imageGeometry ?? widget.session.geometry;
    inputTail = inputTail
        .then((_) async {
          if (ended) return;
          await core.sendRemoteInput(
            id: widget.session.id,
            geometry: display,
            event: jsonEncode({
              'type': type,
              'data': {'id': widget.session.id, ...data},
            }),
          );
        })
        .catchError((Object error) async {
          if (mounted && !ended) {
            ref.read(localProvider.notifier).fail(error);
            await core.stopRemote(id: widget.session.id);
          }
        })
        .whenComplete(() => queued--);
    return inputTail;
  }

  Future<void> releaseHeld() async {
    final heldKeys = keys.toList();
    keys.clear();
    physicalKeys.clear();
    final heldButtons = buttons.toList();
    buttons.clear();
    for (final code in heldKeys) {
      await send('Key', {'code': code, 'down': false});
    }
    for (final button in heldButtons) {
      await send('Button', {'button': button, 'down': false});
    }
  }

  int? keyCode(LogicalKeyboardKey key) {
    final named = {
      LogicalKeyboardKey.enter: 0x110000,
      LogicalKeyboardKey.escape: 0x110001,
      LogicalKeyboardKey.backspace: 0x110002,
      LogicalKeyboardKey.tab: 0x110003,
      LogicalKeyboardKey.controlLeft: 0x110004,
      LogicalKeyboardKey.controlRight: 0x110004,
      LogicalKeyboardKey.shiftLeft: 0x110005,
      LogicalKeyboardKey.shiftRight: 0x110005,
      LogicalKeyboardKey.altLeft: 0x110006,
      LogicalKeyboardKey.altRight: 0x110006,
      LogicalKeyboardKey.metaLeft: 0x110007,
      LogicalKeyboardKey.metaRight: 0x110007,
      LogicalKeyboardKey.arrowLeft: 0x110008,
      LogicalKeyboardKey.arrowRight: 0x110009,
      LogicalKeyboardKey.arrowUp: 0x11000a,
      LogicalKeyboardKey.arrowDown: 0x11000b,
    };
    if (named.containsKey(key)) return named[key];
    if (key.keyId >= 0x20 && key.keyId <= 0x10ffff) return key.keyId;
    final runes = key.keyLabel.runes.toList();
    return runes.length == 1 ? runes.single : null;
  }

  void move(Offset p, Size size, {bool force = false}) {
    final now = DateTime.now();
    if (!force && now.difference(lastMove).inMilliseconds < 30) return;
    lastMove = now;
    unawaited(
      send('Pointer', {
        'x': (p.dx / size.width).clamp(0.0, 1.0),
        'y': (p.dy / size.height).clamp(0.0, 1.0),
      }),
    );
  }

  Future<void> latestFrame(Map<String, dynamic> metadata) async {
    if (loadingFrame) {
      pendingFrame = metadata;
      load++;
      return;
    }
    loadingFrame = true;
    Map<String, dynamic>? next = metadata;
    try {
      while (next != null && !ended) {
        await frame(next);
        next = pendingFrame;
        pendingFrame = null;
      }
    } finally {
      loadingFrame = false;
    }
  }

  Future<void> frame(Map<String, dynamic> metadata) async {
    final ticket = ++load;
    final bytes = await File(metadata['path'] as String).readAsBytes();
    if (!mounted || ended || ticket != load) return;
    if (image != null) await MemoryImage(image!).evict();
    if (!mounted || ended || ticket != load) return;
    setState(() {
      image = bytes;
      final g = metadata['geometry'] as Map<String, dynamic>;
      imageGeometry = Geometry(
        width: g['width'] as int,
        height: g['height'] as int,
        rotation: g['rotation'] as int,
        scale: (g['scale'] as num).toDouble(),
      );
      aspect = (metadata['width'] as num) / (metadata['height'] as num);
    });
  }

  @override
  void dispose() {
    ended = true;
    load++;
    focus.dispose();
    text.dispose();
    unawaited(core.stopRemote(id: widget.session.id).catchError((Object _) {}));
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final state = ref.watch(localProvider);
    final metadata = state.remoteFrames[widget.session.id];
    if (metadata != null && metadata['sequence'] != sequence) {
      sequence = metadata['sequence'] as int;
      unawaited(
        latestFrame(metadata).catchError((Object error) {
          if (mounted && !ended) ref.read(localProvider.notifier).fail(error);
        }),
      );
    }
    if (!ended &&
        state.remoteSessions[widget.session.id]?['status'] == 'ended') {
      ended = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && ModalRoute.of(context)?.isCurrent == true) {
          Navigator.pop(context);
        }
      });
    }
    return Scaffold(
      appBar: AppBar(
        title: Text(
          widget.session.input
              ? 'Approved remote control'
              : 'Approved screen view',
        ),
        actions: [
          TextButton(
            onPressed: () async {
              setState(() => ended = true);
              await core.stopRemote(id: widget.session.id);
              if (context.mounted &&
                  ModalRoute.of(context)?.isCurrent == true) {
                Navigator.pop(context);
              }
            },
            child: const Text('Stop'),
          ),
        ],
      ),
      body: Column(
        children: [
          if (state.error != null)
            Text(state.error!, style: const TextStyle(color: Colors.red)),
          Text(
            android
                ? 'Android · taps, short swipes and focused editable text'
                : 'Primary display · software PNG stream · tap the image to focus keyboard input',
          ),
          Expanded(
            child: Center(
              child: AspectRatio(
                aspectRatio: aspect,
                child: LayoutBuilder(
                  builder: (context, bounds) {
                    final size = Size(bounds.maxWidth, bounds.maxHeight);
                    return Focus(
                      focusNode: focus,
                      onFocusChange: (value) {
                        if (!value) unawaited(releaseHeld());
                      },
                      onKeyEvent: (node, event) {
                        if (!widget.session.input || android) {
                          return KeyEventResult.ignored;
                        }
                        final code = keyCode(event.logicalKey);
                        if (code == null) return KeyEventResult.ignored;
                        final physical = event.physicalKey.usbHidUsage;
                        if (event is KeyDownEvent &&
                            !physicalKeys.containsKey(physical)) {
                          physicalKeys[physical] = code;
                          if (keys.add(code)) {
                            unawaited(
                              send('Key', {'code': code, 'down': true}),
                            );
                          }
                        }
                        if (event is KeyUpEvent) {
                          final released = physicalKeys.remove(physical);
                          if (released != null &&
                              !physicalKeys.containsValue(released) &&
                              keys.remove(released)) {
                            unawaited(
                              send('Key', {'code': released, 'down': false}),
                            );
                          }
                        }
                        return KeyEventResult.handled;
                      },
                      child: MouseRegion(
                        onHover: (event) {
                          if (!android) move(event.localPosition, size);
                        },
                        child: Listener(
                          onPointerMove: (event) {
                            if (android) {
                              swipeEnd = normalized(event.localPosition, size);
                            } else {
                              move(event.localPosition, size);
                            }
                          },
                          onPointerDown: (event) {
                            if (android) {
                              swipeStart = normalized(
                                event.localPosition,
                                size,
                              );
                              swipeEnd = swipeStart;
                              return;
                            }
                            focus.requestFocus();
                            move(event.localPosition, size, force: true);
                            for (final pair in [(1, 0), (2, 1), (4, 2)]) {
                              if (event.buttons & pair.$1 != 0 &&
                                  buttons.add(pair.$2)) {
                                unawaited(
                                  send('Button', {
                                    'button': pair.$2,
                                    'down': true,
                                  }),
                                );
                              }
                            }
                          },
                          onPointerUp: (event) {
                            if (android) {
                              final start = swipeStart;
                              final end = swipeEnd;
                              swipeStart = null;
                              swipeEnd = null;
                              if (start != null && end != null) {
                                if ((end - start).distance > 0.03) {
                                  unawaited(
                                    send('Swipe', {
                                      'x0': start.dx,
                                      'y0': start.dy,
                                      'x1': end.dx,
                                      'y1': end.dy,
                                      'duration_ms': 300,
                                    }),
                                  );
                                } else {
                                  unawaited(
                                    send('Tap', {'x': end.dx, 'y': end.dy}),
                                  );
                                }
                              }
                              return;
                            }
                            for (final button in buttons.toList()) {
                              buttons.remove(button);
                              unawaited(
                                send('Button', {
                                  'button': button,
                                  'down': false,
                                }),
                              );
                            }
                          },
                          onPointerSignal: (event) {
                            if (android) return;
                            if (event is PointerScrollEvent) {
                              unawaited(
                                send('Scroll', {
                                  'x': (event.scrollDelta.dx / 20).clamp(
                                    -100.0,
                                    100.0,
                                  ),
                                  'y': (event.scrollDelta.dy / 20).clamp(
                                    -100.0,
                                    100.0,
                                  ),
                                }),
                              );
                            }
                          },
                          child: ColoredBox(
                            color: Colors.black,
                            child:
                                image == null
                                    ? const Center(
                                      child: CircularProgressIndicator(),
                                    )
                                    : Image.memory(
                                      image!,
                                      fit: BoxFit.contain,
                                      gaplessPlayback: true,
                                      errorBuilder:
                                          (context, error, stack) =>
                                              const Center(
                                                child: Text(
                                                  'Frame decode failed',
                                                  style: TextStyle(
                                                    color: Colors.white,
                                                  ),
                                                ),
                                              ),
                                    ),
                          ),
                        ),
                      ),
                    );
                  },
                ),
              ),
            ),
          ),
          if (widget.session.input)
            Padding(
              padding: const EdgeInsets.all(8),
              child: Row(
                children: [
                  Expanded(
                    child: TextField(
                      controller: text,
                      decoration: const InputDecoration(
                        labelText: 'Send text (maximum 4096 UTF-8 bytes)',
                      ),
                    ),
                  ),
                  FilledButton(
                    onPressed: () {
                      if (utf8.encode(text.text).length <= 4096) {
                        unawaited(send('Text', {'text': text.text}));
                        text.clear();
                      }
                    },
                    child: const Text('Send text'),
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}
