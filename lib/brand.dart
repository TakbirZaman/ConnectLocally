import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

const creatorProfile = 'https://www.linkedin.com/in/takbir-zaman-bhuiyan';

/// A fixed, user-initiated external link. Never opens a peer-supplied URL.
Future<void> openCreatorProfile() async {
  if (Platform.isAndroid) {
    await const MethodChannel('localconnect/platform')
        .invokeMethod<void>('openCreatorProfile');
    return;
  }
  final ProcessResult result;
  if (Platform.isWindows) {
    result = await Process.run('rundll32.exe', [
      'url.dll,FileProtocolHandler', creatorProfile,
    ]);
  } else if (Platform.isLinux) {
    result = await Process.run('xdg-open', [creatorProfile]);
  } else {
    throw UnsupportedError('Open the profile in your browser.');
  }
  if (result.exitCode != 0) {
    throw StateError('No browser could open the profile.');
  }
}

class LocalConnectLogo extends StatelessWidget {
  const LocalConnectLogo({super.key, this.size = 36});
  final double size;

  @override
  Widget build(BuildContext context) => Semantics(
    label: 'LocalConnect logo',
    image: true,
    child: SizedBox.square(
      dimension: size,
      child: CustomPaint(painter: _LinkPainter()),
    ),
  );
}

class _LinkPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    canvas.scale(size.width / 64, size.height / 64);
    canvas.drawRRect(
      RRect.fromRectAndRadius(const Rect.fromLTWH(0, 0, 64, 64), const Radius.circular(16)),
      Paint()..color = const Color(0xff10283f),
    );
    final pen = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 5
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    canvas.drawPath(
      Path()..moveTo(24, 20)..lineTo(17, 20)..lineTo(12, 25)
        ..lineTo(12, 36)..lineTo(18, 42)..lineTo(27, 42)..lineTo(39, 30),
      pen..color = const Color(0xff68a8ff),
    );
    canvas.drawPath(
      Path()..moveTo(40, 44)..lineTo(47, 44)..lineTo(52, 39)
        ..lineTo(52, 28)..lineTo(46, 22)..lineTo(37, 22)..lineTo(25, 34),
      pen..color = const Color(0xff53e0c5),
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant _LinkPainter oldDelegate) => false;
}

class CreatorCredit extends StatelessWidget {
  const CreatorCredit({super.key, this.openProfile = openCreatorProfile});
  final Future<void> Function() openProfile;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
    child: Center(
      child: TextButton(
        style: TextButton.styleFrom(
          foregroundColor: Theme.of(context).colorScheme.onSurfaceVariant,
          minimumSize: const Size(48, 48),
        ),
        onPressed: () async {
          try {
            await openProfile();
          } catch (_) {
            if (!context.mounted) return;
            ScaffoldMessenger.of(context).showSnackBar(SnackBar(
              content: const Text('Could not open LinkedIn. Copy the link and open it in your browser.'),
              action: SnackBarAction(
                label: 'Copy link',
                onPressed: () => Clipboard.setData(const ClipboardData(text: creatorProfile)),
              ),
            ));
          }
        },
        child: const Text(
          'Developed by Takbir Zaman Bhuiyan ↗',
          textAlign: TextAlign.center,
          style: TextStyle(fontSize: 12, letterSpacing: 0.2),
        ),
      ),
    ),
  );
}
