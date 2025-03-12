import re
from datetime import datetime
import matplotlib.pyplot as plt
import matplotlib.dates as mdates
import numpy as np
import sys


def parse_log(filename):
    events = []
    current_state = "not_waiting"  # 初始状态为不等待

    with open(filename, 'r') as f:
        for line in f:
            if 'send_waker' not in line:
                continue

            # 解析时间戳
            time_match = re.match(
                r'(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)', line)
            if not time_match:
                continue
            timestamp = datetime.strptime(
                time_match.group(1), '%Y-%m-%dT%H:%M:%S.%fZ')

            # 提取信号
            signal_match = re.search(r'Signals\((.*?)\)', line)
            signal = signal_match.group(1) if signal_match else "Unknown"

            # 判断事件类型
            if 'wait for' in line:
                events.append({
                    'time': timestamp,
                    'type': 'enter_wait',
                    'state': 'waiting',
                    'signal': signal
                })
                current_state = 'waiting'
            elif any(x in line for x in ['wake by', 'woken before registering', 'woken in registering']):
                if current_state == 'waiting':
                    events.append({
                        'time': timestamp,
                        'type': 'exit_wait',
                        'state': 'not_waiting',
                        'signal': signal
                    })
                    current_state = 'not_waiting'

    return events


def visualize_waker_states(events):
    if not events:
        print("No events found")
        return

    plt.figure(figsize=(15, 8))

    # 设置时间范围
    times = [e['time'] for e in events]
    start_time = min(times)
    end_time = max(times)

    # 绘制状态图
    current_state = 'not_waiting'
    last_time = start_time
    y_positions = {'waiting': 2, 'not_waiting': 1}

    # 为标注创建不同的垂直位置
    annotation_positions = []
    current_position = 2.2  # 起始位置

    for event in events:
        if event['type'] == 'enter_wait':
            # 画一条从不等待到等待的线
            plt.plot([last_time, event['time']],
                     [y_positions['not_waiting'], y_positions['not_waiting']],
                     'b-', linewidth=2)
            plt.plot([event['time'], event['time']],
                     [y_positions['not_waiting'], y_positions['waiting']],
                     'r--', linewidth=2)

            # 添加进入等待状态的信号标注
            plt.annotate(f'↑ wait for {event["signal"]}',
                         xy=(event['time'], y_positions['waiting']),
                         xytext=(0, 10),
                         textcoords='offset points',
                         ha='center',
                         va='bottom',
                         rotation=45,
                         color='red')

            last_time = event['time']
            current_state = 'waiting'

        else:  # exit_wait
            # 画一条从等待到不等待的线
            plt.plot([last_time, event['time']],
                     [y_positions['waiting'], y_positions['waiting']],
                     'r-', linewidth=2)
            plt.plot([event['time'], event['time']],
                     [y_positions['waiting'], y_positions['not_waiting']],
                     'g--', linewidth=2)

            # 添加退出等待状态的信号标注
            plt.annotate(f'↓ wake by {event["signal"]}',
                         xy=(event['time'], y_positions['not_waiting']),
                         xytext=(0, -20),
                         textcoords='offset points',
                         ha='center',
                         va='top',
                         rotation=45,
                         color='green')

            last_time = event['time']
            current_state = 'not_waiting'

    # 画最后一段状态
    plt.plot([last_time, end_time],
             [y_positions[current_state], y_positions[current_state]],
             'b-' if current_state == 'not_waiting' else 'r-',
             linewidth=2)

    # 设置图表样式
    plt.ylim(0.5, 3)  # 扩大y轴范围以容纳标注
    plt.yticks([1, 2], ['Not Waiting', 'Waiting'])
    plt.grid(True, linestyle='--', alpha=0.7)
    plt.gca().xaxis.set_major_formatter(mdates.DateFormatter('%H:%M:%S.%f'))
    plt.xticks(rotation=45)

    # 添加图例
    plt.plot([], [], 'b-', label='Not Waiting State', linewidth=2)
    plt.plot([], [], 'r-', label='Waiting State', linewidth=2)
    plt.plot([], [], 'r--', label='Enter Wait', linewidth=2)
    plt.plot([], [], 'g--', label='Exit Wait', linewidth=2)
    plt.legend(loc='upper right')

    plt.title('Send Waker State Timeline with Signals')
    plt.tight_layout()

    # 保存为SVG格式
    plt.savefig('waker_states.svg', format='svg', bbox_inches='tight')
    plt.close()


if __name__ == "__main__":
    events = parse_log(sys.argv[1])
    visualize_waker_states(events)

    # 打印统计信息
    print("\n状态转换统计:")
    enter_wait = sum(1 for e in events if e['type'] == 'enter_wait')
    exit_wait = sum(1 for e in events if e['type'] == 'exit_wait')
    print(f"进入等待状态: {enter_wait}")
    print(f"退出等待状态: {exit_wait}")
