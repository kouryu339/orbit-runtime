// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ConversationRichContentElement } from '../src/components/rich-content.js';
import { parseWidget } from '../src/content/parser.js';

afterEach(() => document.body.replaceChildren());
async function mount(content: string) {
  const element = new ConversationRichContentElement();
  element.content = content;
  element.locale = 'zh-CN';
  document.body.append(element);
  await element.updateComplete;
  return element;
}
async function check(element: ConversationRichContentElement, selector: string) {
  const input = element.shadowRoot!.querySelector<HTMLInputElement>(selector)!;
  input.checked = true;
  input.dispatchEvent(new Event('change'));
  await element.updateComplete;
}
async function text(element: ConversationRichContentElement, selector: string, value: string) {
  const input = element.shadowRoot!.querySelector<HTMLInputElement>(selector)!;
  input.value = value;
  input.dispatchEvent(new Event('input'));
  await element.updateComplete;
}
describe('widgets', () => {
  it('submits a custom single choice instead of the recommendation', async () => {
    const element = await mount('[select:single | label="格式" | options="PDF,DOCX,其他"]');
    const submitted = vi.fn();
    element.addEventListener('agent-widget-submit', submitted);
    expect(element.shadowRoot!.querySelectorAll('.choice-option')).toHaveLength(3);
    await check(element, '.choice-option input');
    expect(element.shadowRoot!.querySelector('.other-input')).not.toBeNull();
    element.shadowRoot!.querySelector<HTMLInputElement>('.other-input')!.focus();
    await element.updateComplete;
    expect(element.shadowRoot!.querySelector<HTMLButtonElement>('.widgets > button')!.disabled).toBe(true);
    await text(element, '.other-input', '  Markdown  ');
    element.shadowRoot!.querySelector<HTMLButtonElement>('.widgets > button')!.click();
    expect(submitted.mock.calls[0][0].detail.content).toBe('格式: Markdown');
  });
  it('combines recommended and custom multi selections', async () => {
    const element = await mount('[select:multi | label="格式" | options="PDF,DOCX"]');
    const submitted = vi.fn();
    element.addEventListener('agent-widget-submit', submitted);
    await check(element, '.choice-option input');
    await text(element, '.other-input', 'Markdown');
    element.shadowRoot!.querySelector<HTMLButtonElement>('.widgets > button')!.click();
    expect(submitted.mock.calls[0][0].detail.content).toBe('格式: PDF, Markdown');
  });
  it('switches from other to a recommended single answer', async () => {
    const element = await mount('[select:single | label="格式" | options="PDF"]');
    const submitted = vi.fn();
    element.addEventListener('agent-widget-submit', submitted);
    await text(element, '.other-input', 'Markdown');
    await check(element, '.choice-option input');
    element.shadowRoot!.querySelector<HTMLButtonElement>('.widgets > button')!.click();
    expect(submitted.mock.calls[0][0].detail.content).toBe('格式: PDF');
  });
  it('keeps path browsing and manual text submission', async () => {
    const element = await mount('[input:path | label="文件" | accept=".txt"]\n[input:text | label="备注"]');
    element.capabilities = { pickPath: vi.fn(async () => ({ paths: ['C:/report.txt'] })) };
    await element.updateComplete;
    element.shadowRoot!.querySelector<HTMLButtonElement>('.path-row button')!.click();
    await Promise.resolve();
    await element.updateComplete;
    expect(element.shadowRoot!.querySelector<HTMLInputElement>('.path-row input')!.value).toBe('C:/report.txt');
    const submitted = vi.fn();
    element.addEventListener('agent-widget-submit', submitted);
    await text(element, '.widget:last-of-type input', 'Hello');
    element.shadowRoot!.querySelector<HTMLButtonElement>('.widgets > button')!.click();
    expect(submitted.mock.calls[0][0].detail.content).toBe('文件: C:/report.txt\n备注: Hello');
  });
  it('shows confirm selection and prevents expired interaction', async () => {
    const element = await mount('[confirm | label="删除"]');
    element.shadowRoot!.querySelector<HTMLButtonElement>('.choice-row button')!.click();
    await element.updateComplete;
    expect(element.shadowRoot!.querySelector('.choice-row button')!.getAttribute('aria-pressed')).toBe('true');
    element.widgetsExpired = true;
    await element.updateComplete;
    expect(element.shadowRoot!.querySelector('.widgets > button')).toBeNull();
    expect(element.shadowRoot!.querySelector('.widget-status')!.textContent).toContain('已过期');
    expect(element.shadowRoot!.querySelector('.choice-row button')!.getAttribute('aria-pressed')).toBe('true');
  });
  it('submits separated widget groups once and disables every control', async () => {
    const element = await mount('[input:text | label="姓名"]\n\n补充说明\n[input:text | label="备注"]\n结束说明');
    const submitted = vi.fn();
    element.addEventListener('agent-widget-submit', submitted);
    expect(element.shadowRoot!.querySelectorAll('.widgets > button')).toHaveLength(1);
    await text(element, 'input[aria-label="姓名"]', '张三');
    await text(element, 'input[aria-label="备注"]', '已确认');
    const submit = element.shadowRoot!.querySelector<HTMLButtonElement>('.widgets > button')!;
    submit.click();
    await element.updateComplete;
    expect(submitted.mock.calls[0][0].detail.content).toBe('姓名: 张三\n备注: 已确认');
    expect([...element.shadowRoot!.querySelectorAll('input')].every((input) => input.disabled)).toBe(true);
    expect(element.shadowRoot!.querySelector('.widgets > button')).toBeNull();
    expect(element.shadowRoot!.querySelector('.widget-status')!.textContent).toContain('已提交');
    element.widgetsExpired = true;
    await element.updateComplete;
    expect(element.shadowRoot!.querySelector('.widget-status')!.textContent).toContain('已提交');
    submit.click();
    expect(submitted).toHaveBeenCalledTimes(1);
  });
  it('does not recognize removed date and time controls', () => {
    expect(parseWidget('[input:date | label="日期"]')).toBeNull();
    expect(parseWidget('[input:time | label="时间"]')).toBeNull();
  });
});
