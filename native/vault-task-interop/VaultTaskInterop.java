// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Smart Explorer contributors
//
// Interop harness of the task suite rechte-desktop-tresore-task (plan.md D3; strang-tresore.md
// VLT-M5, VLT-M7, VLT-M8, VLT-M14). It reads, writes and checks Cryptomator vaults (format 8) with
// CryptoFS, the library of Cryptomator itself, so the suite can prove that vaults written by se are
// readable by Cryptomator and the other way round. Task suite only: never shipped, CryptoFS comes from
// Maven Central at run time (pom.xml).
//
// Usage: java -jar vault-task-interop.jar <command> [options]
//   version
//   create  --vault DIR --password-file FILE [--cipher SIV_GCM|SIV_CTRMAC] [--from DIR]
//   read    --vault DIR --password-file FILE [--expect MANIFEST.json]
//   write   --vault DIR --password-file FILE [--from DIR] [--delete /PATH ...]
//   unlock  --vault DIR --password-file FILE
//   health  --vault DIR --password-file FILE [--allow RESULT_TYPE ...]
// The password file holds the password in UTF-8; one trailing line break is ignored.
// MANIFEST.json: {"files": {"/folder/name.txt": "<sha256 hex>", ...}}; paths are cleartext paths with "/",
// compared after NFC normalization.
// Output: one JSON object on the last line of stdout.
// Exit codes: 0 ok; 1 difference (read --expect) or a health finding of severity WARN/CRITICAL whose
// type is not allowed; 2 the vault does not unlock (wrong password, unreadable masterkey file);
// 3 usage or I/O error.

import org.cryptomator.cryptofs.CryptoFileSystemProperties;
import org.cryptomator.cryptofs.CryptoFileSystemProvider;
import org.cryptomator.cryptofs.VaultConfig;
import org.cryptomator.cryptofs.health.api.DiagnosticResult;
import org.cryptomator.cryptofs.health.api.HealthCheck;
import org.cryptomator.cryptolib.api.Cryptor;
import org.cryptomator.cryptolib.api.CryptorProvider;
import org.cryptomator.cryptolib.api.Masterkey;
import org.cryptomator.cryptolib.api.MasterkeyLoader;
import org.cryptomator.cryptolib.api.MasterkeyLoadingFailedException;
import org.cryptomator.cryptolib.common.MasterkeyFileAccess;

import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileSystem;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.security.SecureRandom;
import java.text.Normalizer;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.stream.Stream;

public final class VaultTaskInterop {

    static final String CRYPTOFS_VERSION = "2.11.0";
    static final String MASTERKEY_FILE = "masterkey.cryptomator";
    static final String VAULT_CONFIG_FILE = "vault.cryptomator";
    static final URI KEY_ID = URI.create("masterkeyfile:" + MASTERKEY_FILE);
    static final int EXIT_OK = 0;
    static final int EXIT_DIFFERENCE = 1;
    static final int EXIT_UNLOCK = 2;
    static final int EXIT_USAGE = 3;

    private VaultTaskInterop() {
    }

    static final class UsageException extends Exception {
        UsageException(String message) {
            super(message);
        }
    }

    public static void main(String[] args) {
        int code;
        try {
            code = run(args);
        } catch (UsageException error) {
            print(failure("usage", error));
            code = EXIT_USAGE;
        } catch (MasterkeyLoadingFailedException error) {
            print(failure("unlock", error));
            code = EXIT_UNLOCK;
        } catch (Exception error) {
            print(failure("error", error));
            code = EXIT_USAGE;
        }
        System.exit(code);
    }

    static int run(String[] args) throws Exception {
        if (args.length == 0) {
            throw new UsageException("missing command (version, create, read, write, unlock, health)");
        }
        Map<String, List<String>> options = options(args);
        switch (args[0]) {
            case "version":
                return version();
            case "create":
                return create(options);
            case "read":
                return read(options);
            case "write":
                return write(options);
            case "unlock":
                return unlock(options);
            case "health":
                return health(options);
            default:
                throw new UsageException("unknown command " + args[0]);
        }
    }

    // ------------------------------------------------------------------ commands

    static int version() {
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("ok", true);
        String implementation = CryptoFileSystemProvider.class.getPackage().getImplementationVersion();
        out.put("cryptofs", implementation != null ? implementation : CRYPTOFS_VERSION);
        out.put("java", System.getProperty("java.version"));
        print(out);
        return EXIT_OK;
    }

    static int create(Map<String, List<String>> options) throws Exception {
        Path vault = requiredPath(options, "--vault");
        CharSequence password = password(options);
        CryptorProvider.Scheme scheme;
        try {
            scheme = CryptorProvider.Scheme.valueOf(single(options, "--cipher", "SIV_GCM"));
        } catch (IllegalArgumentException error) {
            throw new UsageException("--cipher must be SIV_GCM or SIV_CTRMAC");
        }
        Files.createDirectories(vault);
        SecureRandom random = new SecureRandom();
        Masterkey masterkey = Masterkey.generate(random);
        new MasterkeyFileAccess(new byte[0], random).persist(masterkey, vault.resolve(MASTERKEY_FILE), password);
        MasterkeyLoader loader = keyId -> masterkey.copy();
        CryptoFileSystemProperties properties = CryptoFileSystemProperties.cryptoFileSystemProperties()
                .withKeyLoader(loader)
                .withCipherCombo(scheme)
                .build();
        CryptoFileSystemProvider.initialize(vault, properties, KEY_ID);
        int copied = 0;
        String from = single(options, "--from", null);
        if (from != null) {
            try (FileSystem fileSystem = CryptoFileSystemProvider.newFileSystem(vault, properties)) {
                copied = copyInto(Path.of(from), fileSystem.getPath("/"));
            }
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("ok", true);
        out.put("vault", vault.toString());
        out.put("cipher", scheme.name());
        out.put("files", copied);
        print(out);
        return EXIT_OK;
    }

    static int read(Map<String, List<String>> options) throws Exception {
        Path vault = requiredPath(options, "--vault");
        CharSequence password = password(options);
        Map<String, Object> files = new TreeMap<>();
        Map<String, String> hashes = new TreeMap<>();
        List<String> folders = new ArrayList<>();
        try (FileSystem fileSystem = open(vault, password, true)) {
            Path root = fileSystem.getPath("/");
            Files.walkFileTree(root, new SimpleFileVisitor<>() {
                @Override
                public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attributes) {
                    if (!dir.equals(root)) {
                        folders.add(nfc(dir.toString()));
                    }
                    return FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult visitFile(Path file, BasicFileAttributes attributes) throws IOException {
                    if (attributes.isRegularFile()) {
                        String hash = sha256(file);
                        Map<String, Object> item = new LinkedHashMap<>();
                        item.put("size", attributes.size());
                        item.put("sha256", hash);
                        files.put(nfc(file.toString()), item);
                        hashes.put(nfc(file.toString()), hash);
                    }
                    return FileVisitResult.CONTINUE;
                }
            });
        }
        List<String> differences = new ArrayList<>();
        String expect = single(options, "--expect", null);
        if (expect != null) {
            Object manifest = Json.parse(Files.readString(Path.of(expect), StandardCharsets.UTF_8));
            Map<String, String> expected = new TreeMap<>();
            if (manifest instanceof Map<?, ?> top && top.get("files") instanceof Map<?, ?> listed) {
                for (Map.Entry<?, ?> entry : listed.entrySet()) {
                    expected.put(nfc(String.valueOf(entry.getKey())), String.valueOf(entry.getValue()).toLowerCase());
                }
            } else {
                throw new UsageException("--expect: manifest without an object \"files\"");
            }
            for (Map.Entry<String, String> entry : expected.entrySet()) {
                String actual = hashes.get(entry.getKey());
                if (actual == null) {
                    differences.add("missing " + entry.getKey());
                } else if (!actual.equals(entry.getValue())) {
                    differences.add("content differs " + entry.getKey());
                }
            }
            for (String path : hashes.keySet()) {
                if (!expected.containsKey(path)) {
                    differences.add("unexpected " + path);
                }
            }
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("ok", differences.isEmpty());
        out.put("files", files);
        out.put("folders", folders);
        out.put("differences", differences);
        print(out);
        return differences.isEmpty() ? EXIT_OK : EXIT_DIFFERENCE;
    }

    static int write(Map<String, List<String>> options) throws Exception {
        Path vault = requiredPath(options, "--vault");
        CharSequence password = password(options);
        int copied = 0;
        List<String> deleted = new ArrayList<>();
        try (FileSystem fileSystem = open(vault, password, false)) {
            String from = single(options, "--from", null);
            if (from != null) {
                copied = copyInto(Path.of(from), fileSystem.getPath("/"));
            }
            for (String target : options.getOrDefault("--delete", List.of())) {
                Path path = fileSystem.getPath(target);
                if (Files.exists(path)) {
                    try (Stream<Path> tree = Files.walk(path)) {
                        for (Path item : tree.sorted(Comparator.reverseOrder()).toList()) {
                            Files.delete(item);
                        }
                    }
                    deleted.add(target);
                }
            }
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("ok", true);
        out.put("files", copied);
        out.put("deleted", deleted);
        print(out);
        return EXIT_OK;
    }

    static int unlock(Map<String, List<String>> options) throws Exception {
        Path vault = requiredPath(options, "--vault");
        CharSequence password = password(options);
        Masterkey masterkey = loadMasterkey(vault, password);
        VaultConfig config = verifiedConfig(vault, masterkey);
        try (FileSystem fileSystem = open(vault, password, true)) {
            Files.exists(fileSystem.getPath("/"));
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("ok", true);
        out.put("cipher", config.getCipherCombo().name());
        out.put("vaultVersion", config.getVaultVersion());
        out.put("shorteningThreshold", config.getShorteningThreshold());
        print(out);
        return EXIT_OK;
    }

    static int health(Map<String, List<String>> options) throws Exception {
        Path vault = requiredPath(options, "--vault");
        CharSequence password = password(options);
        List<String> allowed = options.getOrDefault("--allow", List.of());
        Masterkey masterkey = loadMasterkey(vault, password);
        VaultConfig config = verifiedConfig(vault, masterkey);
        Cryptor cryptor = CryptorProvider.forScheme(config.getCipherCombo()).provide(masterkey.copy(), new SecureRandom());
        List<Map<String, Object>> findings = new ArrayList<>();
        List<String> checks = new ArrayList<>();
        for (HealthCheck check : HealthCheck.allChecks()) {
            checks.add(check.name());
            check.check(vault, config, masterkey, cryptor, result -> findings.add(finding(check, result)));
        }
        boolean unexpected = false;
        for (Map<String, Object> finding : findings) {
            String severity = String.valueOf(finding.get("severity"));
            boolean serious = severity.equals("WARN") || severity.equals("CRITICAL");
            if (serious && !allowed.contains(String.valueOf(finding.get("type")))) {
                unexpected = true;
            }
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("ok", !unexpected);
        out.put("checks", checks);
        out.put("findings", findings);
        out.put("allowed", allowed);
        print(out);
        return unexpected ? EXIT_DIFFERENCE : EXIT_OK;
    }

    // ------------------------------------------------------------------ vault helpers

    static FileSystem open(Path vault, CharSequence password, boolean readOnly) throws Exception {
        MasterkeyFileAccess access = new MasterkeyFileAccess(new byte[0], new SecureRandom());
        MasterkeyLoader loader = keyId -> access.load(vault.resolve(MASTERKEY_FILE), password);
        CryptoFileSystemProperties.Builder builder = CryptoFileSystemProperties.cryptoFileSystemProperties()
                .withKeyLoader(loader);
        if (readOnly) {
            builder = builder.withFlags(CryptoFileSystemProperties.FileSystemFlags.READONLY);
        }
        return CryptoFileSystemProvider.newFileSystem(vault, builder.build());
    }

    static Masterkey loadMasterkey(Path vault, CharSequence password) {
        return new MasterkeyFileAccess(new byte[0], new SecureRandom()).load(vault.resolve(MASTERKEY_FILE), password);
    }

    static VaultConfig verifiedConfig(Path vault, Masterkey masterkey) throws Exception {
        String token = Files.readString(vault.resolve(VAULT_CONFIG_FILE), StandardCharsets.US_ASCII).trim();
        VaultConfig.UnverifiedVaultConfig unverified = VaultConfig.decode(token);
        return unverified.verify(masterkey.getEncoded(), unverified.allegedVaultVersion());
    }

    static Map<String, Object> finding(HealthCheck check, DiagnosticResult result) {
        Map<String, Object> item = new LinkedHashMap<>();
        item.put("check", check.name());
        item.put("type", result.getClass().getSimpleName());
        item.put("severity", result.getSeverity().name());
        item.put("text", String.valueOf(result));
        return item;
    }

    static int copyInto(Path source, Path targetRoot) throws IOException {
        if (!Files.isDirectory(source)) {
            throw new IOException("--from is not a folder: " + source);
        }
        int[] copied = {0};
        Files.walkFileTree(source, new SimpleFileVisitor<>() {
            @Override
            public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attributes) throws IOException {
                Files.createDirectories(target(dir));
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult visitFile(Path file, BasicFileAttributes attributes) throws IOException {
                if (attributes.isRegularFile()) {
                    Files.copy(file, target(file), StandardCopyOption.REPLACE_EXISTING);
                    copied[0]++;
                }
                return FileVisitResult.CONTINUE;
            }

            private Path target(Path path) {
                Path result = targetRoot;
                for (Path part : source.relativize(path)) {
                    if (!part.toString().isEmpty()) {
                        result = result.resolve(part.toString());
                    }
                }
                return result;
            }
        });
        return copied[0];
    }

    static String sha256(Path file) throws IOException {
        MessageDigest digest;
        try {
            digest = MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException error) {
            throw new IOException(error);
        }
        try (InputStream in = Files.newInputStream(file)) {
            byte[] buffer = new byte[1 << 16];
            for (int read = in.read(buffer); read >= 0; read = in.read(buffer)) {
                digest.update(buffer, 0, read);
            }
        }
        return HexFormat.of().formatHex(digest.digest());
    }

    static String nfc(String text) {
        return Normalizer.normalize(text, Normalizer.Form.NFC);
    }

    // ------------------------------------------------------------------ arguments

    static Map<String, List<String>> options(String[] args) throws UsageException {
        Map<String, List<String>> options = new HashMap<>();
        for (int index = 1; index < args.length; index += 2) {
            if (!args[index].startsWith("--") || index + 1 >= args.length) {
                throw new UsageException("expected --option value pairs after the command, got " + args[index]);
            }
            options.computeIfAbsent(args[index], key -> new ArrayList<>()).add(args[index + 1]);
        }
        return options;
    }

    static String single(Map<String, List<String>> options, String name, String fallback) {
        List<String> values = options.get(name);
        return values == null || values.isEmpty() ? fallback : values.get(values.size() - 1);
    }

    static Path requiredPath(Map<String, List<String>> options, String name) throws UsageException {
        String value = single(options, name, null);
        if (value == null) {
            throw new UsageException(name + " is required");
        }
        return Path.of(value);
    }

    static CharSequence password(Map<String, List<String>> options) throws UsageException, IOException {
        String text = Files.readString(requiredPath(options, "--password-file"), StandardCharsets.UTF_8);
        if (text.endsWith("\r\n")) {
            text = text.substring(0, text.length() - 2);
        } else if (text.endsWith("\n")) {
            text = text.substring(0, text.length() - 1);
        }
        return text;
    }

    // ------------------------------------------------------------------ output

    static Map<String, Object> failure(String kind, Exception error) {
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("ok", false);
        out.put("kind", kind);
        out.put("error", error.getClass().getSimpleName() + ": " + error.getMessage());
        return out;
    }

    static void print(Map<String, Object> value) {
        System.out.println(Json.write(value));
        System.out.flush();
    }

    /** Minimal JSON writer and reader (objects, arrays, strings, numbers, booleans, null). */
    static final class Json {
        private final String text;
        private int at;

        private Json(String text) {
            this.text = text;
        }

        static String write(Object value) {
            StringBuilder out = new StringBuilder();
            write(value, out);
            return out.toString();
        }

        private static void write(Object value, StringBuilder out) {
            if (value == null) {
                out.append("null");
            } else if (value instanceof Boolean || value instanceof Number) {
                out.append(value);
            } else if (value instanceof Map<?, ?> map) {
                out.append('{');
                boolean first = true;
                for (Map.Entry<?, ?> entry : map.entrySet()) {
                    if (!first) {
                        out.append(',');
                    }
                    first = false;
                    writeString(String.valueOf(entry.getKey()), out);
                    out.append(':');
                    write(entry.getValue(), out);
                }
                out.append('}');
            } else if (value instanceof List<?> list) {
                out.append('[');
                for (int index = 0; index < list.size(); index++) {
                    if (index > 0) {
                        out.append(',');
                    }
                    write(list.get(index), out);
                }
                out.append(']');
            } else {
                writeString(String.valueOf(value), out);
            }
        }

        private static void writeString(String value, StringBuilder out) {
            out.append('"');
            for (int index = 0; index < value.length(); index++) {
                char c = value.charAt(index);
                switch (c) {
                    case '"' -> out.append("\\\"");
                    case '\\' -> out.append("\\\\");
                    case '\n' -> out.append("\\n");
                    case '\r' -> out.append("\\r");
                    case '\t' -> out.append("\\t");
                    default -> {
                        if (c < 0x20) {
                            out.append(String.format("\\u%04x", (int) c));
                        } else {
                            out.append(c);
                        }
                    }
                }
            }
            out.append('"');
        }

        static Object parse(String text) throws UsageException {
            Json parser = new Json(text);
            Object value = parser.value();
            parser.space();
            if (parser.at != text.length()) {
                throw new UsageException("JSON: trailing characters at " + parser.at);
            }
            return value;
        }

        private void space() {
            while (at < text.length() && Character.isWhitespace(text.charAt(at))) {
                at++;
            }
        }

        private Object value() throws UsageException {
            space();
            if (at >= text.length()) {
                throw new UsageException("JSON: unexpected end");
            }
            char c = text.charAt(at);
            if (c == '{') {
                return object();
            }
            if (c == '[') {
                return array();
            }
            if (c == '"') {
                return string();
            }
            if (text.startsWith("true", at)) {
                at += 4;
                return Boolean.TRUE;
            }
            if (text.startsWith("false", at)) {
                at += 5;
                return Boolean.FALSE;
            }
            if (text.startsWith("null", at)) {
                at += 4;
                return null;
            }
            int start = at;
            while (at < text.length() && "+-0123456789.eE".indexOf(text.charAt(at)) >= 0) {
                at++;
            }
            if (start == at) {
                throw new UsageException("JSON: unexpected character at " + at);
            }
            return Double.parseDouble(text.substring(start, at));
        }

        private Map<String, Object> object() throws UsageException {
            Map<String, Object> map = new LinkedHashMap<>();
            at++;
            space();
            if (at < text.length() && text.charAt(at) == '}') {
                at++;
                return map;
            }
            while (true) {
                space();
                if (at >= text.length() || text.charAt(at) != '"') {
                    throw new UsageException("JSON: expected a key at " + at);
                }
                String key = string();
                space();
                if (at >= text.length() || text.charAt(at) != ':') {
                    throw new UsageException("JSON: expected ':' at " + at);
                }
                at++;
                map.put(key, value());
                space();
                if (at < text.length() && text.charAt(at) == ',') {
                    at++;
                    continue;
                }
                if (at < text.length() && text.charAt(at) == '}') {
                    at++;
                    return map;
                }
                throw new UsageException("JSON: expected ',' or '}' at " + at);
            }
        }

        private List<Object> array() throws UsageException {
            List<Object> list = new ArrayList<>();
            at++;
            space();
            if (at < text.length() && text.charAt(at) == ']') {
                at++;
                return list;
            }
            while (true) {
                list.add(value());
                space();
                if (at < text.length() && text.charAt(at) == ',') {
                    at++;
                    continue;
                }
                if (at < text.length() && text.charAt(at) == ']') {
                    at++;
                    return list;
                }
                throw new UsageException("JSON: expected ',' or ']' at " + at);
            }
        }

        private String string() throws UsageException {
            StringBuilder out = new StringBuilder();
            at++;
            while (at < text.length()) {
                char c = text.charAt(at++);
                if (c == '"') {
                    return out.toString();
                }
                if (c != '\\') {
                    out.append(c);
                    continue;
                }
                if (at >= text.length()) {
                    break;
                }
                char escaped = text.charAt(at++);
                switch (escaped) {
                    case 'n' -> out.append('\n');
                    case 'r' -> out.append('\r');
                    case 't' -> out.append('\t');
                    case 'b' -> out.append('\b');
                    case 'f' -> out.append('\f');
                    case 'u' -> {
                        if (at + 4 > text.length()) {
                            throw new UsageException("JSON: short \\u escape");
                        }
                        out.append((char) Integer.parseInt(text.substring(at, at + 4), 16));
                        at += 4;
                    }
                    default -> out.append(escaped);
                }
            }
            throw new UsageException("JSON: unterminated string");
        }
    }
}
