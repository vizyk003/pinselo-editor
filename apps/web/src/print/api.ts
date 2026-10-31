/** Print orders API client (A18). */
import type { PreflightResult, PriceBreakdown, PrintCatalog, ShippingAddress } from '@oc/engine';
import { ApiError } from '../api';

async function req<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, { method, credentials: 'same-origin', headers: body === undefined ? {} : { 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) });
  const ct = res.headers.get('content-type') ?? '';
  const j = ct.includes('application/json') ? ((await res.json()) as T & { error?: string; message?: string }) : null;
  if (!res.ok) throw Object.assign(new ApiError(res.status, j?.error ?? 'error', j?.message), { body: j });
  return (j ?? (await res.blob())) as T;
}

export type OrderStatus = 'pending_payment' | 'paid' | 'rendering' | 'submitted' | 'in_production' | 'shipped' | 'delivered' | 'returned' | 'failed' | 'cancelled';
export interface Tracking { carrier?: string; code?: string; url?: string }
export interface PrintOrder {
  id: string; number: string; email: string; design_id: string | null; design_title: string; product_id: string; product_name: string; size_id: string;
  options: Record<string, string>; quantity: number; address: ShippingAddress; shipping_method: string; currency: string;
  subtotal_cents: number; shipping_cents: number; tax_cents: number; total_cents: number; price: PriceBreakdown; preflight: PreflightResult | null;
  status: OrderStatus; provider: string | null; provider_order_id: string | null; payment_provider: string | null; refunded: boolean; hasFile?: boolean; file_sha?: string | null;
  tracking: Tracking[]; error: string | null; paid_at: string | null; created_at: string; updated_at: string; next?: OrderStatus[];
}
export interface OrderEvent { status: OrderStatus; note: string; source: string; created_at: string; actor?: string | null }
export interface SavedAddress { id: string; address: ShippingAddress; is_default: boolean }
export interface CatalogInfo { catalog: PrintCatalog; currency: string; payment: 'stripe' | 'local'; fulfilment: string[] }
export interface Selection { productId: string; sizeId: string; options: Record<string, string>; quantity: number }

export const printApi = {
  catalog: () => req<CatalogInfo>('GET', '/api/print/catalog'),
  quote: (b: Selection & { shippingMethod?: string; country?: string; state?: string }) => req<{ quote: PriceBreakdown }>('POST', '/api/print/quote', b),
  preflight: (b: { designId: string; productId: string; sizeId: string; options: Record<string, string> }) => req<{ preflight: PreflightResult }>('POST', '/api/print/preflight', b),
  addresses: () => req<{ addresses: SavedAddress[]; countries: string[] }>('GET', '/api/print/addresses'),
  saveAddress: (address: Partial<ShippingAddress>, id?: string, makeDefault?: boolean) =>
    req<{ address: SavedAddress }>(id ? 'PUT' : 'POST', id ? `/api/print/addresses/${id}` : '/api/print/addresses', { address, default: makeDefault }),
  deleteAddress: (id: string) => req('DELETE', `/api/print/addresses/${id}`),
  order: (b: Selection & { designId: string; shippingMethod: string; addressId: string; acknowledgeWarnings: boolean }) => req<{ order: PrintOrder; url: string }>('POST', '/api/print/orders', b),
  orders: () => req<{ orders: PrintOrder[] }>('GET', '/api/print/orders'),
  get: (id: string) => req<{ order: PrintOrder; events: OrderEvent[]; canCancel: boolean }>('GET', `/api/print/orders/${id}`),
  pay: (id: string) => req<{ url: string }>('POST', `/api/print/orders/${id}/pay`),
  localPay: (id: string, session: string) => req('POST', `/api/print/orders/${id}/local-pay`, { session }),
  cancel: (id: string) => req('POST', `/api/print/orders/${id}/cancel`),
  fileUrl: (id: string) => `/api/print/orders/${id}/file`,

  admin: {
    orders: (status?: string, q?: string) => req<{ orders: PrintOrder[]; counts: Record<string, number>; fulfilment: string[] }>('GET', `/api/admin/print/orders?${new URLSearchParams({ ...(status ? { status } : {}), ...(q ? { q } : {}) })}`),
    get: (id: string) => req<{ order: PrintOrder; events: OrderEvent[] }>('GET', `/api/admin/print/orders/${id}`),
    setStatus: (id: string, b: { status: OrderStatus; note?: string; tracking?: Tracking; refund?: boolean }) => req<{ order: PrintOrder }>('POST', `/api/admin/print/orders/${id}/status`, b),
    retry: (id: string) => req('POST', `/api/admin/print/orders/${id}/retry`),
    fileUrl: (id: string) => `/api/admin/print/orders/${id}/file`,
  },
};

export function money(cents: number, currency: string): string {
  try {
    return new Intl.NumberFormat(document.documentElement.lang || undefined, { style: 'currency', currency: currency.toUpperCase() }).format(cents / 100);
  } catch {
    return `${(cents / 100).toFixed(2)} ${currency.toUpperCase()}`;
  }
}
