from http.server import HTTPServer, BaseHTTPRequestHandler
import json
import time

class MockProviderHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        print("\n--- REQUEST START ---", flush=True)
        time.sleep(0.1) # Simulate network latency
        content_length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_length).decode('utf-8')
        
        print(f"\n[MOCK PROVIDER] Received {self.path} via {self.command}", flush=True)
        # Use a safer way to print headers to avoid encoding issues on Windows console
        try:
            for key, value in self.headers.items():
                print(f"  {key}: {value}", flush=True)
        except:
            print("  (Some headers could not be printed due to encoding)", flush=True)
        
        print(f"Body: {body}", flush=True)
        
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        
        response = {
            "status": "success",
            "mock_response": "This is a response from the mock provider",
            "received_headers": dict(self.headers)
        }
        self.wfile.write(json.dumps(response).encode('utf-8'))

    def do_GET(self):
        self.do_POST()

def run(server_class=HTTPServer, handler_class=MockProviderHandler, port=8085):
    server_address = ('', port)
    httpd = server_class(server_address, handler_class)
    print(f"Mock Provider listening on port {port}...")
    httpd.serve_forever()

if __name__ == '__main__':
    run()
