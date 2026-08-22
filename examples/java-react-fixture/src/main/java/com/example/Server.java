// A plain JDK-only backend (com.sun.net.httpserver.HttpServer, no
// framework/dependency) serving a built React SPA (frontend/dist) as
// static assets, plus a JSON /api/health route -- the "Backend .jar +
// Static Web Assets" output shape from docs/ROADMAP.md's Java + React/Node
// row. Like examples/rust-react-fixture and examples/go-react-fixture,
// this isn't a composite pipeline: paws ci --toolchain java (this
// package) and paws ci --toolchain node (frontend/) are two independent
// runs against the same repo, proving the existing java/node toolchains
// already compose cleanly for this shape with no new paws capability
// needed.
package com.example;

import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

public class Server {
    private static final Path STATIC_ROOT = Path.of("frontend/dist");

    public static HttpServer create(int port) throws IOException {
        HttpServer server = HttpServer.create(new InetSocketAddress(port), 0);
        server.createContext("/api/health", exchange -> {
            byte[] body = "{\"status\":\"ok\"}".getBytes(StandardCharsets.UTF_8);
            exchange.getResponseHeaders().set("Content-Type", "application/json");
            exchange.sendResponseHeaders(200, body.length);
            try (OutputStream os = exchange.getResponseBody()) {
                os.write(body);
            }
        });
        server.createContext("/", exchange -> {
            String requested = exchange.getRequestURI().getPath();
            Path file = STATIC_ROOT.resolve(requested.equals("/") ? "index.html" : requested.substring(1)).normalize();
            if (!file.startsWith(STATIC_ROOT) || !Files.isRegularFile(file)) {
                exchange.sendResponseHeaders(404, -1);
                return;
            }
            byte[] body = Files.readAllBytes(file);
            exchange.sendResponseHeaders(200, body.length);
            try (OutputStream os = exchange.getResponseBody()) {
                os.write(body);
            }
        });
        return server;
    }

    public static void main(String[] args) throws IOException {
        HttpServer server = create(3000);
        server.start();
    }
}
