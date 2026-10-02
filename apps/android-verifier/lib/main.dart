import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'verifier.dart';

// Android inactive may mean only a focus change while still visible.
// Kotlin onStop remains the independent, direct native cancellation boundary.
bool lifecycleRequiresCancellation(AppLifecycleState state) => switch (state) {
  AppLifecycleState.hidden ||
  AppLifecycleState.paused ||
  AppLifecycleState.detached => true,
  AppLifecycleState.inactive || AppLifecycleState.resumed => false,
};

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const VerifierApp());
}

class VerifierApp extends StatelessWidget {
  const VerifierApp({super.key});
  @override
  Widget build(BuildContext context) => MaterialApp(
    debugShowCheckedModeBanner: false,
    title: 'Nexa 设备验证',
    theme: ThemeData(
      colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xff315969)),
      useMaterial3: true,
    ),
    home: const VerifierPage(),
  );
}

class VerifierPage extends StatefulWidget {
  const VerifierPage({super.key});
  @override
  State<VerifierPage> createState() => _VerifierPageState();
}

class _VerifierPageState extends State<VerifierPage>
    with WidgetsBindingObserver {
  final verifier = Verifier();
  final window = TextWindow();
  Map<String, dynamic>? snapshot;
  Map<String, dynamic>? terminal;
  String? operation;
  String status = '正在初始化真实 MNN 桥接…';
  String? error;
  bool busy = false;
  bool visible = true;
  int pollEpoch = 0;
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(initialize());
  }

  Future<void> initialize() async {
    try {
      final state = await verifier.initialize();
      if (!mounted) return;
      setState(() {
        snapshot = state;
        status = '就绪';
        terminal = state['latest_terminal'] as Map<String, dynamic>?;
      });
      final active = state['active_operation'];
      if (active != null) unawaited(poll(active['operation_id'] as String));
    } catch (e) {
      showError(e);
    }
  }

  void showError(Object e) {
    if (!mounted) return;
    setState(() {
      error = switch (e) {
        VerifierFailure(:final code) => code,
        PlatformException(:final code) => code,
        _ => 'bridge_unavailable',
      };
      busy = false;
    });
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    visible = !lifecycleRequiresCancellation(state);
    if (!visible) {
      window.clear();
      if (mounted) setState(() {});
      final id = operation;
      if (id != null) unawaited(verifier.cancel(id).catchError((Object _) {}));
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    pollEpoch++;
    final id = operation;
    if (id != null) unawaited(verifier.cancel(id).catchError((Object _) {}));
    window.clear();
    super.dispose();
  }

  Future<void> action(Future<Map<String, dynamic>?> Function() run) async {
    if (busy || operation != null) return;
    setState(() {
      busy = true;
      error = null;
    });
    try {
      final ref = await run();
      if (!mounted) return;
      if (ref != null) {
        window.clear();
        window.lastSequence = 0;
        terminal = null;
        await poll(ref['operation_id'] as String);
      } else {
        setState(() => busy = false);
      }
    } catch (e) {
      showError(e);
    }
  }

  Future<void> poll(String id) async {
    final generation = ++pollEpoch;
    operation = id;
    busy = false;
    int? ack;
    int last = 0;
    if (mounted) setState(() {});
    try {
      while (mounted && generation == pollEpoch) {
        final reply = await verifier.next(id, ack);
        if (!mounted || generation != pollEpoch) return;
        final event = reply['event'] as Map<String, dynamic>?;
        if (event != null) {
          final sequence = event['sequence'] as int;
          if (sequence > last) {
            final payload = event['payload'] as Map<String, dynamic>;
            switch (event['kind']) {
              case 'text_delta':
                if (visible) window.append(sequence, payload['text'] as String);
              case 'progress':
                status = payload['stage'] as String;
              case 'case_started':
                status = '${payload['layer']} · ${payload['case_id']}';
              case 'case_result':
                status = '${payload['case_id']}: ${payload['verdict']}';
            }
            last = sequence;
          }
          ack = sequence;
        }
        final finished = reply['terminal'] as Map<String, dynamic>?;
        if (finished != null) {
          final latest = await verifier.snapshot();
          if (!mounted || generation != pollEpoch) return;
          terminal = finished;
          operation = null;
          snapshot = latest;
          status = '${finished['outcome']} · cleanup=${finished['cleanup']}';
          setState(() {});
          return;
        }
        setState(() {});
      }
    } catch (e) {
      showError(e);
      operation = null;
    }
  }

  Future<void> exportReport() async {
    final id = terminal?['operation_id'];
    if (id == null) return;
    try {
      final result = await verifier.export(id as String);
      if (mounted) {
        setState(() => status = result == 'saved' ? '报告已保存到你选择的位置' : '已取消保存');
      }
    } catch (e) {
      showError(e);
    }
  }

  @override
  Widget build(BuildContext context) {
    final ready = snapshot?['candidate']?['state'] == 'ready';
    final idle =
        !busy &&
        operation == null &&
        snapshot != null &&
        snapshot?['host_state'] != 'cleanup_unconfirmed';
    return Scaffold(
      appBar: AppBar(
        title: const Text('Nexa 设备验证'),
        actions: [
          IconButton(
            tooltip: '第三方许可',
            onPressed: () => verifier.licenses(),
            icon: const Icon(Icons.description_outlined),
          ),
        ],
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(20),
          children: [
            const Text(
              'B3a · CPU 研究包',
              style: TextStyle(fontSize: 23, fontWeight: FontWeight.bold),
            ),
            const SizedBox(height: 8),
            const Text(
              '仅用于设备研究。research_only=true · production_admitted=false\n不会自动授予生产准入。无聊天数据库、网络下载或后台生成。',
            ),
            const SizedBox(height: 16),
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('模型：${ready ? '固定候选已就绪' : '未导入'}'),
                    const Text('Qwen3-0.6B · MNN · arm64 · CPU / 2线程 / 2048'),
                    Text('状态：$status'),
                    if (error != null)
                      Text(
                        '错误：$error',
                        style: TextStyle(
                          color: Theme.of(context).colorScheme.error,
                        ),
                      ),
                    if (snapshot != null)
                      Text(
                        '设备：${snapshot!['device']['manufacturer']} ${snapshot!['device']['model']} · ${snapshot!['device']['page_size']} byte pages',
                      ),
                  ],
                ),
              ),
            ),
            if (snapshot == null && error != null)
              OutlinedButton(
                onPressed: initialize,
                child: const Text('重新读取初始化状态'),
              ),
            const SizedBox(height: 12),
            FilledButton.icon(
              onPressed: idle && !ready
                  ? () => action(verifier.importCandidate)
                  : null,
              icon: const Icon(Icons.folder_open),
              label: const Text('选择固定候选的五个文件'),
            ),
            const Text(
              'config.json、llm_config.json、llm.mnn、llm.mnn.weight、tokenizer.txt\n需要约 995 MiB 可用空间；只读源文件，复制到应用私有存储。',
              style: TextStyle(fontSize: 12),
            ),
            const SizedBox(height: 12),
            FilledButton(
              onPressed: idle && ready
                  ? () => action(() => verifier.start('b3a_smoke_v1'))
                  : null,
              child: const Text('运行真实 smoke'),
            ),
            OutlinedButton(
              onPressed: idle && ready
                  ? () => action(() => verifier.start('b3a_safety_v1'))
                  : null,
              child: const Text('运行安全用例（人工项另记未测）'),
            ),
            const OutlinedButton(
              onPressed: null,
              child: Text('长期 stability 尚未实现'),
            ),
            if (operation != null)
              FilledButton.tonal(
                onPressed: () async {
                  try {
                    await verifier.cancel(operation!);
                    if (mounted) {
                      setState(() => status = 'stopping：等待原生安全返回与卸载');
                    }
                  } catch (e) {
                    showError(e);
                  }
                },
                child: const Text('停止，等待安全卸载'),
              ),
            OutlinedButton(
              onPressed: idle && ready ? () => action(verifier.remove) : null,
              child: const Text('移除应用内候选副本'),
            ),
            if (terminal != null)
              OutlinedButton.icon(
                onPressed: exportReport,
                icon: const Icon(Icons.save_alt),
                label: const Text('导出报告（TXT，内容为JSON）'),
              ),
            const Divider(),
            const Text(
              '实时片段（不保存正文）',
              style: TextStyle(fontWeight: FontWeight.bold),
            ),
            if (window.truncated) const Text('仅保留近期输出（64 KiB上限）'),
            SelectableText(
              window.text.isEmpty ? '运行时在这里显示真实流式输出' : window.text,
            ),
            const SizedBox(height: 20),
            const Text(
              '切到后台会立即请求原生取消，安全返回后卸载；回到前台不会重放。停止不等于内核已返回。报告不包含正文、URI、序列号或日志。',
              style: TextStyle(fontSize: 12),
            ),
          ],
        ),
      ),
    );
  }
}
