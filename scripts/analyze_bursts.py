from datetime import datetime
import re
import sys


class Burst:
    def __init__(self, start_time, quota, last_end=None):
        self.start_time = start_time
        self.loaded_time = None
        self.sent_time = None
        self.quota = quota
        self.bytes_transferred = 0
        self.status = "Unknown"
        self.wait_reason = None
        if last_end:
            self.void = start_time - last_end
        else:
            self.void = None

    def total_duration(self):
        if self.sent_time:
            return (self.sent_time - self.start_time).total_seconds() * 1000
        return None

    def socket_time(self):
        if self.loaded_time and self.sent_time:
            return (self.sent_time - self.loaded_time).total_seconds() * 1000
        return None

    def load_time(self):
        if self.loaded_time:
            return (self.loaded_time - self.start_time).total_seconds() * 1000
        return None

    def void_time(self):
        if self.void:
            return self.void.total_seconds() * 1000
        return None

    def __str__(self):
        output = [
            f"初始配额: {self.quota}"
            f"状态: {self.status}",
            f"持续时间: {self.total_duration()} ms",
            f"空隙时间: {self.void_time()} ms",
        ]

        if self.status == "Success":
            output.append(f"传输数据量: {self.bytes_transferred} bytes"
                          f"加载时间: {self.load_time()} ms"
                          f"Socket时间: {self.socket_time()} ms",)
        elif self.status == "Failed":
            output.append(f"等待原因: {self.wait_reason}")

        return "\n".join(output)


class BurstAnalyzer:
    def __init__(self):
        self.bursts = []
        self.current_burst = None
        self.last_end = None

    def parse_timestamp(self, line):
        try:
            match = re.search(
                r'(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)', line)
            if match:
                return datetime.strptime(match.group(1), '%Y-%m-%dT%H:%M:%S.%fZ')
        except Exception:
            pass
        return None

    def analyze_line(self, line):
        timestamp = self.parse_timestamp(line)
        if not timestamp:
            return

        # 检测burst开始
        if 'burst: get quota:' in line:
            quota = int(re.search(r'quota: (\d+)', line).group(1))
            if self.current_burst:
                print(f"Warning: 未处理的burst: {self.current_burst}")
                sys.exit(1)
            self.current_burst = Burst(timestamp, quota, self.last_end)

        if self.current_burst:
            # 检测load结果
            load_match = re.search(r'burst: load_result=Ok\((\d+)\)', line)
            if load_match:
                self.current_burst.bytes_transferred += int(
                    load_match.group(1))

            # 检测加载完成
            if 'burst: loaded segments' in line:
                self.current_burst.loaded_time = timestamp

            # 检测burst成功结束
            if 'burst: sent all' in line:
                self.current_burst.status = "Success"
                self.current_burst.sent_time = timestamp
                self.bursts.append(self.current_burst)
                self.current_burst = None
                self.last_end = timestamp

            # 检测burst失败
            elif 'wait for' in line:
                self.current_burst.status = "Failed"
                self.current_burst.wait_reason = line.split(
                    'wait for')[-1].strip()
                self.current_burst.sent_time = timestamp
                self.bursts.append(self.current_burst)
                self.current_burst = None
                self.last_end = timestamp

    def print_analysis(self):
        print("\nBurst 分析报告:")
        print("-" * 50)

        successful_bursts = [b for b in self.bursts if b.status == "Success"]
        failed_bursts = [b for b in self.bursts if b.status == "Failed"]

        for i, burst in enumerate(self.bursts, 1):
            print(f"\nBurst #{i}:")
            print(str(burst))

        print("\n总结:")
        print(f"总burst数: {len(self.bursts)}")
        print(f"成功的burst数: {len(successful_bursts)}")
        print(f"失败的burst数: {len(failed_bursts)}")
        if successful_bursts:
            total_bytes = sum(b.bytes_transferred for b in successful_bursts)
            print(f"总传输数据量: {total_bytes} bytes")


def analyze_log_file(file_path):
    analyzer = BurstAnalyzer()

    with open(file_path, 'r') as f:
        for line in f:
            analyzer.analyze_line(line)

    # 处理最后一个可能未结束的burst
    if analyzer.current_burst:
        analyzer.bursts.append(analyzer.current_burst)

    analyzer.print_analysis()


if __name__ == "__main__":
    analyze_log_file(sys.argv[1])
