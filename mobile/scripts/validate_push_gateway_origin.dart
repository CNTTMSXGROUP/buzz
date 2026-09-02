import 'dart:io';

bool isValidPushGatewayOrigin(String value) {
  try {
    final uri = Uri.parse(value);
    if (uri.scheme != 'http' && uri.scheme != 'https') return false;
    if (!uri.hasAuthority || uri.host.isEmpty || uri.userInfo.isNotEmpty) {
      return false;
    }
    if (uri.path.isNotEmpty && uri.path != '/') return false;
    if (uri.hasQuery || uri.hasFragment) return false;
    final port = uri.port;
    return port >= 1 && port <= 65535;
  } on FormatException {
    return false;
  }
}

void main(List<String> arguments) {
  if (arguments.length == 1 && isValidPushGatewayOrigin(arguments.single)) {
    return;
  }
  stderr.writeln(
    'error: BUZZ_PUSH_GATEWAY_URL must be an HTTP(S) origin without credentials, path, query, or fragment.',
  );
  exitCode = 1;
}
