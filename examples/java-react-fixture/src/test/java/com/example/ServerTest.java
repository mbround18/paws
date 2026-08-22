package com.example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import com.sun.net.httpserver.HttpServer;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import org.junit.jupiter.api.Test;

class ServerTest {
    @Test
    void healthRouteReturnsOk() throws Exception {
        HttpServer server = Server.create(0);
        server.start();
        try {
            int port = server.getAddress().getPort();
            HttpClient client = HttpClient.newHttpClient();
            HttpRequest request =
                    HttpRequest.newBuilder(URI.create("http://127.0.0.1:" + port + "/api/health"))
                            .build();
            HttpResponse<String> response =
                    client.send(request, HttpResponse.BodyHandlers.ofString());

            assertEquals(200, response.statusCode());
            assertEquals("{\"status\":\"ok\"}", response.body());
        } finally {
            server.stop(0);
        }
    }
}
