import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import App from '../src/App';
import { DesktopController } from '../src/controller';
import { modelTestStatus } from '../src/modelTestStatus';
import type { LocalValidation } from '../src/types';
import { StatusBar } from '../src/StatusBar';
import { deferred, makeApi, model } from './fixtures';
import { configuredSnapshot, revision, modelConfiguration } from './configurationFixtures';

async function mount(page: 'api' | 'models' | 'settings' = 'models', value = configuredSnapshot()) {
  const api = makeApi({ snapshot: vi.fn(async () => value) });
  const controller = new DesktopController(api);
  render(<App initialPage={page} controller={controller} />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  return { api, controller, user: userEvent.setup() };
}

describe('compact UI navigation and test-evidence regressions (mock only)', () => {
  it('F01 keeps API dialog confirmation focused across a runtime refresh', async () => {
    const { api, controller } = await mount('api');
    fireEvent.click(screen.getByRole('button', { name: '复制 API 令牌' }));
    const confirm = within(screen.getByRole('dialog')).getByRole('button', { name: '确认复制' });
    confirm.focus();
    await act(async () => controller.refresh());
    expect(confirm).toHaveFocus();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(api.copyToken).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
  it('F02 retains download view and typed filter through settings navigation', async () => {
    const { api } = await mount('models', configuredSnapshot(true));
    fireEvent.click(screen.getByRole('button', { name: '下载模型' }));
    fireEvent.change(screen.getByRole('searchbox', { name: '筛选下载目录' }), { target: { value: 'Qwen' } });
    fireEvent.click(screen.getByRole('button', { name: '目录与下载源设置' }));
    fireEvent.click(screen.getByRole('button', { name: '模型库' }));
    expect(screen.getByRole('button', { name: '下载模型' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('searchbox', { name: '筛选下载目录' })).toHaveValue('Qwen');
    expect(api.downloadStart).not.toHaveBeenCalled();
  });
  it('F03 opens model detail with focus and returns to the originating detail button', async () => {
    const { user } = await mount();
    const button = screen.getByRole('button', { name: `查看 ${model.display_name} 的详情` });
    button.focus();
    await user.keyboard('{Enter}');
    expect(screen.getByRole('heading', { level: 1, name: model.display_name })).toHaveFocus();
    expect(screen.getByRole('button', { name: `复制 ${model.display_name} 的模型 ID` })).toBeEnabled();
    await user.click(screen.getByRole('button', { name: /返回模型库/ }));
    expect(screen.getByRole('button', { name: `查看 ${model.display_name} 的详情` })).toHaveFocus();
    expect(screen.queryByText(/API ID：/)).not.toBeInTheDocument();
  });
  it('F04 selected-but-unloaded stays neutral, has no residency or loaded action', async () => {
    const value = configuredSnapshot(); value.runtime!.state = 'unloaded';
    await mount('models', value);
    expect(screen.getByLabelText('应用状态栏')).toHaveTextContent('无驻留模型');
    expect(screen.queryByText('当前驻留')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: '加载模型' })).toBeEnabled();
    expect(screen.getByRole('article', { name: model.display_name })).toHaveClass('test-neutral');
  });
  it('F05 dirty settings stay visible in collapsed group and preserve revision conflicts', async () => {
    let value = configuredSnapshot(true);
    const api = makeApi({ snapshot: vi.fn(async () => value), configurationSave: vi.fn() });
    const controller = new DesktopController(api);
    render(<App initialPage="settings" controller={controller} />);
    await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
    const title = screen.getByText('运行默认值');
    const group = title.closest('details')!;
    fireEvent.click(title);
    fireEvent.change(screen.getByRole('spinbutton', { name: '默认上下文长度' }), { target: { value: '8192' } });
    await waitFor(() => expect(within(group.querySelector('summary')!).getByText('未保存')).toBeVisible());
    fireEvent.click(title);
    expect(group).not.toHaveAttribute('open');
    value = structuredClone(value); value.configuration!.revision = revision('b'); value.configuration!.saved.global_defaults.context_size = 16384;
    await act(async () => controller.refresh());
    await waitFor(() => expect(within(group.querySelector('summary')!).getByText('版本变化 · 待核对')).toBeVisible());
    fireEvent.click(title);
    expect(screen.getByRole('spinbutton', { name: '默认上下文长度' })).toHaveValue(8192);
    expect(screen.getByRole('button', { name: '保存全局默认值' })).toBeDisabled();
    expect(api.configurationSave).not.toHaveBeenCalled();
  });
  it('F06 model profile draft survives detail, navigation and return without saving', async () => {
    const value = configuredSnapshot();
    const api = makeApi({ snapshot: vi.fn(async () => value), configurationModelGet: vi.fn(async () => modelConfiguration()), configurationSave: vi.fn() });
    const controller = new DesktopController(api);
    render(<App initialPage="models" controller={controller} />);
    await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: `查看 ${model.display_name} 的详情` }));
    fireEvent.click(screen.getByText('运行档案与当前参数'));
    fireEvent.change(await screen.findByRole('spinbutton', { name: /模型上下文长度/ }), { target: { value: '8192' } });
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(screen.getByRole('button', { name: '模型库' }));
    expect(screen.getByRole('heading', { level: 1, name: model.display_name })).toBeInTheDocument();
    fireEvent.click(screen.getByText('运行档案与当前参数'));
    expect(await screen.findByRole('spinbutton', { name: /模型上下文长度/ })).toHaveValue(8192);
    expect(api.configurationSave).not.toHaveBeenCalled();
  });
  it.each([
    ['passed', true, true, null, 'passed'],
    ['loaded', true, false, null, 'neutral'],
    ['untested', false, false, null, 'neutral'],
    ['failed', false, false, 'model_load_failed', 'failed'],
    ['failed', true, false, 'request_cancelled', 'neutral'],
    ['failed', true, false, 'validation_record_write_failed', 'neutral'],
    ['stale', true, true, null, 'neutral'],
    ['deferred', false, false, 'runtime_busy', 'neutral'],
    ['unavailable', false, false, 'validation_record_invalid', 'neutral'],
  ] as const)('F07 test tone respects %s/%s/%s/%s', (state, load_success, generation_pass, error_code, tone) => {
    const value = { state, load_success, generation_pass, error_code, checked_at_unix_ms: 1 } satisfies LocalValidation;
    expect(modelTestStatus(value).tone).toBe(tone);
  });
  it('F08 new running attempt takes precedence over old passed history', () => {
    const value = { state: 'passed', load_success: true, generation_pass: true, error_code: null, checked_at_unix_ms: 1 } satisfies LocalValidation;
    expect(modelTestStatus(value, { id: 2, model_id: model.id, model_signature: 'test', mode: 'test', phase: 'running', started_at: 2, finished_at: null, result: null, error: null })).toMatchObject({ tone: 'neutral', running: true });
  });
  it('cancels an unconfirmed dirty-draft close on Alt navigation without closing or losing the draft', async () => {
    const { api, user } = await mount('settings', configuredSnapshot(true));
    fireEvent.click(screen.getByText('运行默认值'));
    fireEvent.change(screen.getByRole('spinbutton', { name: '默认上下文长度' }), { target: { value: '8192' } });
    const trigger = screen.getByRole('button', { name: '关闭窗口并保留服务' });
    await user.click(trigger);
    const dialog = screen.getByRole('dialog');
    const cancel = within(dialog).getByRole('button', { name: '取消' });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(document, { key: '1', altKey: true });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '概览', level: 1 })).toHaveFocus();
    expect(api.close).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(screen.getByText('运行默认值'));
    expect(screen.getByRole('spinbutton', { name: '默认上下文长度' })).toHaveValue(8192);
  });
  it('returns to the model-list title when a refresh removes the detail origin', async () => {
    let rows = [model];
    const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot(true)), modelsPage: vi.fn(async () => ({ data: rows, generation: 'generation-1', next_after: null })) });
    const controller = new DesktopController(api);
    const user = userEvent.setup();
    render(<App initialPage="models" controller={controller} />);
    await user.click(await screen.findByRole('button', { name: `查看 ${model.display_name} 的详情` }));
    rows = [];
    await act(async () => controller.refreshModels());
    expect(screen.getByRole('heading', { name: '模型已不在当前列表' })).toBeVisible();
    expect(screen.getByRole('heading', { name: '模型已不在当前列表' })).toHaveFocus();
    await user.click(screen.getByRole('button', { name: /返回模型库/ }));
    expect(screen.getByRole('heading', { name: '模型库', level: 1 })).toHaveFocus();
    expect(screen.queryByRole('button', { name: `查看 ${model.display_name} 的详情` })).not.toBeInTheDocument();
    expect(api.loadModel).not.toHaveBeenCalled();
  });
  it('shows and dismisses each repeated off-page test failure without hiding the next identical result', async () => {
    let pending = deferred<LocalValidation>();
    const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot()), testModel: vi.fn(() => pending.promise) });
    const controller = new DesktopController(api);
    const user = userEvent.setup();
    render(<App initialPage="models" controller={controller} />);
    for (let attempt = 1; attempt <= 2; attempt++) {
      await user.click(await screen.findByRole('button', { name: `查看 ${model.display_name} 的详情` }));
      await user.click(screen.getByRole('button', { name: `测试 ${model.display_name}` }));
      await user.click(screen.getByRole('button', { name: 'API 接入' }));
      await act(async () => pending.resolve({ state: 'failed', load_success: true, generation_pass: false, checked_at_unix_ms: attempt, error_code: 'deadline_exceeded' }));
      await waitFor(() => expect(controller.getSnapshot().testing_model).toBeNull());
      expect(screen.getByRole('alert')).toHaveTextContent('短文本测试失败');
      await user.click(screen.getByRole('button', { name: '收起操作结果' }));
      expect(controller.getSnapshot().notice).toBeNull();
      await act(async () => controller.refresh());
      expect(screen.queryByRole('button', { name: '收起操作结果' })).not.toBeInTheDocument();
      pending = deferred<LocalValidation>();
      await user.click(screen.getByRole('button', { name: '模型库' }));
      expect(screen.getByText('本次基础测试失败')).toBeVisible();
      await user.click(screen.getByRole('button', { name: /返回模型库/ }));
    }
    expect(api.testModel).toHaveBeenCalledTimes(2);
  });
  it('deduplicates a terminal notice only while that same current model detail is visible', async () => {
    const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot()), testModel: vi.fn(async () => ({ state: 'failed' as const, load_success: true, generation_pass: false, checked_at_unix_ms: 2, error_code: 'deadline_exceeded' })) });
    const controller = new DesktopController(api);
    const user = userEvent.setup();
    render(<App initialPage="models" controller={controller} />);
    await user.click(await screen.findByRole('button', { name: `查看 ${model.display_name} 的详情` }));
    await user.click(screen.getByRole('button', { name: `测试 ${model.display_name}` }));
    await waitFor(() => expect(controller.getSnapshot().testing_model).toBeNull());
    expect(screen.getByText('本次基础测试失败')).toBeVisible();
    expect(screen.queryByRole('button', { name: '收起操作结果' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'API 接入' }));
    expect(screen.getByRole('alert')).toHaveTextContent('短文本测试失败');
  });
  it.each(['stale', 'unavailable', 'failed'] as const)('invalidates a finished pass when native reread changes to %s', async (state) => {
    let proof: LocalValidation = { state: 'passed', load_success: true, generation_pass: true, checked_at_unix_ms: 1, error_code: null };
    const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot()), modelsPage: vi.fn(async () => ({ data: [{ ...model, local_validation: proof }], generation: 'generation-1', next_after: null })), testModel: vi.fn(async () => { proof = { ...proof, checked_at_unix_ms: 2 }; return proof; }) });
    const controller = new DesktopController(api);
    const user = userEvent.setup();
    render(<App initialPage="models" controller={controller} />);
    await user.click(await screen.findByRole('button', { name: `查看 ${model.display_name} 的详情` }));
    await user.click(screen.getByRole('button', { name: `测试 ${model.display_name}` }));
    await waitFor(() => expect(controller.getSnapshot().testing_model).toBeNull());
    await user.click(screen.getByRole('button', { name: /返回模型库/ }));
    expect(screen.getByRole('article', { name: model.display_name })).toHaveClass('test-passed');
    proof = { state, load_success: state !== 'unavailable', generation_pass: state === 'stale', checked_at_unix_ms: state === 'stale' ? 2 : 3, error_code: state === 'unavailable' ? 'validation_record_read_failed' : state === 'failed' ? 'deadline_exceeded' : null };
    await act(async () => controller.refreshModels());
    expect(screen.getByRole('article', { name: model.display_name })).toHaveClass(`test-${state === 'failed' ? 'failed' : 'neutral'}`);
    await user.click(screen.getByRole('button', { name: `查看 ${model.display_name} 的详情` }));
    const feedback = screen.getByLabelText('本次模型测试');
    expect(feedback).toHaveClass(state === 'failed' ? 'failed' : 'neutral');
    expect(feedback).not.toHaveClass('passed');
    expect(within(feedback).getByRole('status')).toHaveTextContent('最新本机记录');
  });
  it.each([
    ['passed', 1, 'stale', 1, null, 'neutral'],
    ['passed', 1, 'unavailable', null, 'validation_record_read_failed', 'neutral'],
    ['passed', 1, 'failed', 2, 'deadline_exceeded', 'failed'],
    ['failed', 2, 'passed', 1, null, 'failed'],
    ['passed', 2, 'failed', 1, 'deadline_exceeded', 'passed'],
    ['failed', 1, 'passed', 2, null, 'passed'],
    ['passed', 1, 'failed', 1, 'validation_record_write_failed', 'neutral'],
    ['passed', 1, 'failed', 1, 'validation_refresh_failed', 'neutral'],
  ] as const)('reconciles finished %s at %s against native %s at %s with %s', (attemptState, attemptTime, evidenceState, evidenceTime, evidenceCode, tone) => {
    const result: LocalValidation = { state: attemptState, load_success: true, generation_pass: attemptState === 'passed', checked_at_unix_ms: attemptTime, error_code: attemptState === 'failed' ? 'deadline_exceeded' : null };
    const evidence: LocalValidation = { state: evidenceState, load_success: evidenceState !== 'unavailable', generation_pass: ['passed', 'stale'].includes(evidenceState), checked_at_unix_ms: evidenceTime, error_code: evidenceCode };
    const attempt = { id: 1, model_id: model.id, model_signature: 'same', mode: 'test' as const, phase: 'finished' as const, started_at: 0, finished_at: attemptTime, result, error: null };
    expect(modelTestStatus(evidence, attempt).tone).toBe(tone);
    expect(modelTestStatus(evidence, { ...attempt, phase: 'running' }).tone).toBe('neutral');
  });
  it('opens download directory and source controls, and returns with the filter and saved target intact', async () => {
    const value = configuredSnapshot(true);
    value.model_directory = { state: 'stopped', configured: { directory_id: 'dir-1', display_path: 'D:\\下载模型', library_generation: 'generation-1' }, effective: null };
    const { api } = await mount('models', value);
    fireEvent.click(screen.getByRole('button', { name: '下载模型' }));
    fireEvent.change(screen.getByRole('searchbox', { name: '筛选下载目录' }), { target: { value: 'Qwen' } });
    expect(screen.getByText('下载到：D:\\下载模型')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: '目录与下载源设置' }));
    expect(screen.getByRole('button', { name: '选择模型目录' })).toBeVisible();
    const source = screen.getByRole('combobox', { name: '默认下载源' });
    expect(source).toBeVisible();
    fireEvent.change(source, { target: { value: 'huggingface' } });
    fireEvent.click(screen.getByRole('button', { name: /返回下载/ }));
    expect(screen.getByRole('button', { name: '下载模型' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('searchbox', { name: '筛选下载目录' })).toHaveValue('Qwen');
    expect(screen.getByText('下载源：ModelScope')).toBeVisible();
    expect(screen.getByText('下载到：D:\\下载模型')).toBeVisible();
    expect(api.downloadStart).not.toHaveBeenCalled();
    expect(api.configureDirectory).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '目录与下载源设置' }));
    expect(screen.getByRole('combobox', { name: '默认下载源' })).toHaveValue('huggingface');
    expect(screen.getByText('未保存')).toBeVisible();
  });
  it('keeps collapsed settings collapsed through a snapshot poll without resetting focus', async () => {
    const { controller } = await mount('settings', configuredSnapshot(true));
    const title = screen.getByText('运行默认值');
    const group = title.closest('details')!;
    expect(group.open).toBe(false);
    fireEvent.click(title);
    expect(group.open).toBe(true);
    await act(async () => controller.refresh());
    expect(group.open).toBe(true);
    fireEvent.click(title);
    group.querySelector('summary')!.focus();
    await act(async () => controller.refresh());
    expect(group.open).toBe(false);
    expect(group.querySelector('summary')).toHaveFocus();
  });
  it('keeps the global footer outside page content and reflects other clients generating', async () => {
    const value = configuredSnapshot();
    value.runtime!.state = 'generating';
    value.runtime!.active_request = 'other-client-request';
    const { api } = await mount('models', value);
    const footer = screen.getByLabelText('应用状态栏');
    expect(footer.tagName).toBe('FOOTER');
    expect(screen.getByRole('main')).not.toContainElement(footer);
    expect(footer).toHaveTextContent(model.display_name);
    expect(footer).toHaveTextContent('模型正在生成');
    expect(footer).not.toHaveTextContent('无进行中操作');
    expect(screen.getByText('当前有任务，空闲后可加载或切换模型')).toBeVisible();
    fireEvent.click(within(footer).getByRole('button', { name: '查看活动' }));
    expect(screen.getByRole('heading', { name: '活动', level: 1 })).toHaveFocus();
    expect(screen.getByLabelText('应用状态栏')).toHaveTextContent('模型正在生成');
    expect(api.chatCancel).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it.each(['loading', 'unloading', 'faulted'] as const)('does not invent residency for the %s state', (state) => {
    const value = configuredSnapshot();
    value.runtime!.state = state;
    const controller = new DesktopController(makeApi());
    render(<StatusBar state={{ ...controller.getSnapshot(), booting: false, snapshot: value }} goActivity={() => {}} />);
    expect(screen.getByLabelText('应用状态栏')).not.toHaveTextContent(model.display_name);
    expect(screen.getByLabelText('应用状态栏')).not.toHaveTextContent('模型已就绪');
  });
  it('keeps primary navigation shortcuts out of editing and IME composition', async () => {
    await mount('models', configuredSnapshot(true));
    fireEvent.click(screen.getByRole('button', { name: '下载模型' }));
    const input = screen.getByRole('searchbox', { name: '筛选下载目录' });
    input.focus();
    fireEvent.keyDown(input, { key: '5', altKey: true });
    expect(screen.getByRole('heading', { name: '模型库', level: 1 })).toBeVisible();
    fireEvent.keyDown(document, { key: '5', altKey: true, isComposing: true });
    expect(screen.getByRole('heading', { name: '模型库', level: 1 })).toBeVisible();
    fireEvent.keyDown(document, { key: '5', altKey: true });
    expect(screen.getByRole('heading', { name: '设置', level: 1 })).toHaveFocus();
  });
  it.each([
    ['passed', true, true, null, 'passed'],
    ['failed', true, false, 'deadline_exceeded', 'failed'],
    ['stale', true, true, null, 'neutral'],
    ['loaded', true, false, null, 'neutral'],
    ['failed', true, false, 'request_cancelled', 'neutral'],
    ['unavailable', false, false, 'validation_record_invalid', 'neutral'],
  ] as const)('renders %s/%s/%s/%s evidence using only the appropriate row tone', async (state, load_success, generation_pass, error_code, tone) => {
    const proof = { state, load_success, generation_pass, error_code, checked_at_unix_ms: 1 } satisfies LocalValidation;
    const api = makeApi({ modelsPage: vi.fn(async () => ({ data: [{ ...model, local_validation: proof }], next_after: null, generation: 'generation-1' })) });
    const controller = new DesktopController(api);
    render(<App initialPage="models" controller={controller} />);
    const row = await screen.findByRole('article', { name: model.display_name });
    expect(row).toHaveClass(`test-${tone}`);
    expect(row.querySelectorAll('.model-test-status')).toHaveLength(1);
    for (const other of ['passed', 'failed', 'neutral'].filter((candidate) => candidate !== tone)) expect(row).not.toHaveClass(`test-${other}`);
  });
});
