import os
import subprocess
import re


class Benchmark:

    def __init__(self):
        pass

    class ServerRunner:
        impl_name: str
        launch_server: list[str]
        listen_port: int

        def __init__(self, impl_name: str, launch_server: list[str], listen_port: int):
            self.impl_name = impl_name
            self.listen_port = listen_port
            self.launch_server = launch_server

        def run(self) -> subprocess.Popen:
            # 在后台运行server
            return subprocess.Popen(
                self.launch_server,
                cwd=rand_files.path,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )

    class Result:
        success: int
        duration: float
        qps: float

        def __init__(self, success: int, duration: float):
            self.success, self.duration = success, duration
            self.qps = success / duration if duration > 0 else 0

        @staticmethod
        def average(results: list['Benchmark.Result']) -> 'Benchmark.Result':
            total_success = sum(result.success for result in results)
            total_duration = sum(result.duration for result in results)
            return Benchmark.Result(total_success, total_duration)


root = os.path.join(os.path.dirname(__file__), "..")


class RandomFiles:
    path = os.path.join(root, "rand_files")

    def __init__(self):
        if not os.path.exists(self.path):
            os.makedirs(self.path)

    def gen(self, file_size: int) -> str:
        file_name = f"rand_file_{file_size}.bin"
        file_path = os.path.join(self.path, file_name)
        if not os.path.exists(file_path):
            with open(file_path, "wb") as f:
                f.write(os.urandom(int(file_size) * 1024))
        return file_name


class Certs:
    path = os.path.join(root, "certs")

    root_cert = os.path.join(path, "root_cert.pem")
    root_key = os.path.join(path, "root_key.pem")
    server_cert = os.path.join(path, "server_cert.pem")
    server_key = os.path.join(path, "server_key.pem")
    server_csr = os.path.join(path, "server_csr.pem")
    server_cert_der = os.path.join(path, "server_cert.der")
    server_key_der = os.path.join(path, "server_key.der")

    def __init__(self):
        if not os.path.exists(self.path):
            os.makedirs(self.path)
            # CA
            subprocess.run(
                ["openssl", "ecparam", "-name", "prime256v1",
                    "-genkey", "-out", self.root_key])
            subprocess.run(
                ["openssl", "req", "-new", "-x509", "-key", self.root_key, "-out", self.root_cert,
                 "-days", "3650", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost"])
            # Server
            subprocess.run(
                ["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-out", self.server_key])
            subprocess.run(
                ["openssl", "req", "-new", "-key", self.server_key, "-out",
                 self.server_csr,  "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost"])
            subprocess.run(
                ["openssl", "x509", "-req", "-in", self.server_csr, "-CA", self.root_cert,
                 "-CAkey", self.root_key, "-CAcreateserial", "-out", self.server_cert,
                 "-days", "365", "-copy_extensions", "copy"])
            # Convert pem to der
            subprocess.run(
                ["openssl", "x509", "-in", self.server_cert, "-outform", "der",
                 "-out", self.server_cert_der])
            subprocess.run(
                ["openssl", "ec", "-in", self.server_key, "-outform", "der",
                 "-out", self.server_key_der])


rand_files = RandomFiles()
ecc_certs = Certs()

go_quic_dir = os.path.join(root, "go-quic")
gm_quic_dir = os.path.join(root, "gm-quic")
tquic_dir = os.path.join(root, "tquic")
quinn_dir = os.path.join(root, "h3")
quiche_dir = os.path.join(root, "quiche")


def go_quic_runner() -> Benchmark.ServerRunner:
    # 编译
    subprocess.run(
        ["go", "build", "-ldflags=-s -w", "-trimpath", "-o", "quic_server"],
        cwd=go_quic_dir
    )

    binary = os.path.join(go_quic_dir, "quic_server")
    launch = [binary,
              "-a", "[::1]:4430",
              "-c", ecc_certs.server_cert,
              "-k", ecc_certs.server_key,]

    return Benchmark.ServerRunner('go-quic', launch, 4430)


def git_clone(owner: str, repo: str, branch: str) -> None:
    if not os.path.exists(repo):
        subprocess.run(["git", "clone", "--depth", "1", "--recursive", "--branch",
                       branch, f"https://github.com/{owner}/{repo}"],
                       cwd=root)


def gm_quic_runner() -> Benchmark.ServerRunner:

    git_clone("genmeta", "gm-quic", "main")

    # 编译
    subprocess.run(
        ["cargo", "build", "--release", "--package",
            "h3-shim", "--example", "h3-server"],
        cwd=gm_quic_dir
    )

    binary = os.path.join(gm_quic_dir,
                          "target", "release", "examples", "h3-server")

    launch = [
        binary,
        "-c", ecc_certs.server_cert,
        "-k", ecc_certs.server_key,
        "-l", "[::1]:4431"
    ]

    return Benchmark.ServerRunner('gm-quic', launch, 4431)


def tquic_runner() -> Benchmark.ServerRunner:
    git_clone("Tencent", "tquic", "v1.6.0")

    subprocess.run(
        ["cargo", "build", "--release", "--package",
            "tquic_tools", "--bin", "tquic_server"],
        cwd=tquic_dir
    )

    binary = os.path.join(tquic_dir, "target", "release", "tquic_server")

    launch = [
        binary,
        "-c", ecc_certs.server_cert,
        "-k", ecc_certs.server_key,
        "-l", "[::1]:4432",
    ]

    return Benchmark.ServerRunner('tquic', launch, 4432)


def quinn_runner() -> Benchmark.ServerRunner:
    git_clone("hyperium", "h3", "h3-quinn-v0.0.9")

    subprocess.run(
        ["cargo", "build", "--release", "--example", "server"],
        cwd=quinn_dir
    )

    binary = os.path.join(quinn_dir,
                          "target", "release", "examples", "server")

    launch = [
        binary,
        "-c", ecc_certs.server_cert_der,
        "-k", ecc_certs.server_key_der,
        "-l", "[::1]:4433",
        "-d", "./"  # 实际上是rand-files
    ]

    return Benchmark.ServerRunner('quinn', launch, 4433)


def quiche_runner() -> Benchmark.ServerRunner:
    git_clone("cloudflare", "quiche", "0.23.4")

    subprocess.run(
        ["cargo", "build", "--release", "--bin", "quiche-server"],
        cwd=quiche_dir
    )

    binary = os.path.join(quiche_dir,
                          "target", "release", "quiche-server")

    launch = [
        binary,
        "--key", ecc_certs.server_key,
        "--cert", ecc_certs.server_cert,
        "--listen", "[::1]:4434",
        "--no-retry"
    ]

    return Benchmark.ServerRunner('quiche', launch, 4434)


class H3Client:
    connections: int
    requests: int

    def __init__(self, conenctions: int = 512, requests: int = 128):
        self.connections = conenctions
        self.requests = requests

    def run_once(self, server_runner: Benchmark.ServerRunner, file_size: int) -> Benchmark.Result:
        # 在后台启动server
        server = server_runner.run()

        uri = f'https://localhost:{server_runner.listen_port}/{rand_files.gen(file_size)}'
        result = subprocess.run(
            ["cargo", "run",
             "--release", "--bin", "h3-client", "--",
             "-c", str(self.connections), "-r", str(self.requests), "--roots", ecc_certs.root_cert, uri],
            env={**os.environ, "RUST_LOG": "info"},
            stdout=subprocess.PIPE,
            text=True
        )

        server.kill()

        # Extract total_time and success_queries using regex
        output = result.stdout
        match = re.search(
            r"success_queries=(\d+).*?total_time=(\d+\.?\d*)", output)
        if match:
            success_queries = int(match.group(1))
            total_time = float(match.group(2))
            return Benchmark.Result(success=int(success_queries), duration=total_time)
        else:
            raise ValueError(f'Failed to parse benchmark output: {output}')

    def run_many(self, server_runner: Benchmark.ServerRunner, file_size: int, times: int = 3) -> list[Benchmark]:
        results = []
        for _ in range(0, times):
            results.append(self.run_once(server_runner, file_size))
        return results


if __name__ == "__main__":
    runners = [
        go_quic_runner(),
        gm_quic_runner(),
        tquic_runner(),
        quinn_runner(),
        quiche_runner()
    ]

    client = H3Client(conenctions=512, requests=64)

    results = {}

    for runner in runners:
        runner_results = []
        for file_size in [15, 30, 2048]:
            runner_results.insert(file_size, client.run_many(3))
        results.insert(runner.impl_name, runner_results)

    print(results)
