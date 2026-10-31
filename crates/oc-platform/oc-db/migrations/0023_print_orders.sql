-- Print orders (A18): saved shipping addresses, orders with a price snapshot and preflight report, a status
-- history, and idempotency for fulfiller webhooks.

CREATE TABLE print_addresses (
  id            uuid PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  address       jsonb NOT NULL,                       -- ShippingAddress (validated, normalized)
  is_default    boolean NOT NULL DEFAULT false,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX print_addresses_user ON print_addresses(user_id);

CREATE TABLE print_orders (
  id                uuid PRIMARY KEY,
  number            text NOT NULL UNIQUE,              -- customer-facing reference, also the fulfiller external id
  user_id           uuid REFERENCES users(id) ON DELETE SET NULL,
  email             text NOT NULL,
  design_id         text REFERENCES designs(id) ON DELETE SET NULL,
  design_title      text NOT NULL DEFAULT '',
  product_id        text NOT NULL,
  product_name      text NOT NULL,
  size_id           text NOT NULL,
  options           jsonb NOT NULL DEFAULT '{}',
  quantity          integer NOT NULL CHECK (quantity > 0),
  address           jsonb NOT NULL,
  shipping_method   text NOT NULL,
  currency          text NOT NULL,
  subtotal_cents    integer NOT NULL,
  shipping_cents    integer NOT NULL,
  tax_cents         integer NOT NULL,
  total_cents       integer NOT NULL,
  price             jsonb NOT NULL,                    -- PriceBreakdown at order time
  preflight         jsonb,                             -- PreflightResult at order time
  status            text NOT NULL DEFAULT 'pending_payment'
                    CHECK (status IN ('pending_payment','paid','rendering','submitted','in_production','shipped','delivered','returned','failed','cancelled')),
  provider          text,                              -- gelato | printful | local
  provider_order_id text,
  payment_provider  text,                              -- stripe | local
  payment_ref       text,                              -- Stripe checkout session / payment intent
  refunded          boolean NOT NULL DEFAULT false,
  file_sha          text,                              -- rendered print file (object store)
  file_type         text,
  tracking          jsonb NOT NULL DEFAULT '[]',       -- [{ carrier, code, url }]
  error             text,
  paid_at           timestamptz,
  created_at        timestamptz NOT NULL DEFAULT now(),
  updated_at        timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX print_orders_user ON print_orders(user_id, created_at DESC);
CREATE INDEX print_orders_status ON print_orders(status, created_at DESC);
CREATE INDEX print_orders_provider ON print_orders(provider, provider_order_id) WHERE provider_order_id IS NOT NULL;

CREATE TABLE print_order_events (
  id          bigserial PRIMARY KEY,
  order_id    uuid NOT NULL REFERENCES print_orders(id) ON DELETE CASCADE,
  status      text NOT NULL,
  note        text NOT NULL DEFAULT '',
  source      text NOT NULL CHECK (source IN ('customer','staff','provider','payment','system')),
  actor_id    uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX print_order_events_order ON print_order_events(order_id, id);

CREATE TABLE print_webhook_events (
  id          text PRIMARY KEY,                        -- provider:event id (or body hash)
  provider    text NOT NULL,
  received_at timestamptz NOT NULL DEFAULT now()
);
