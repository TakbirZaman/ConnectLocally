import 'package:flutter/material.dart';

Future<bool> showPhoneControlDisclosure(BuildContext context) async =>
    await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Allow phone control with Accessibility?'),
        content: const SingleChildScrollView(
          child: Text(
            'LocalConnect uses Android Accessibility to perform taps, swipes, '
            'and text entry sent by a paired device during a remote-control '
            'session you approve. It accesses the focused editable field to '
            'insert text; it does not upload the accessibility view tree.\n\n'
            'Screen images are shared separately over your local network using '
            'Android screen-capture consent. They may include private information '
            'visible on your screen. Only approve devices and people you trust.\n\n'
            'No screen images, typed text, or files are sent to the developer '
            'or a cloud server. Stop session or Stop all ends remote access. '
            'You can also disable the service in Android Accessibility settings.\n\n'
            'Accessibility is optional. File transfer and screen viewing work '
            'without it. Continue opens Android settings; you choose whether '
            'to enable the service there.',
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Not now'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Agree and open settings'),
          ),
        ],
      ),
    ) ?? false;
