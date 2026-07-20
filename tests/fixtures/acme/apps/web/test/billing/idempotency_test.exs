defmodule AcmeWeb.Billing.IdempotencyTest do
  use ExUnit.Case, async: true

  alias AcmeWeb.Billing.Charge

  test "repeated create with the same idempotency_key collapses to one charge" do
    attrs = %{amount_cents: 1299, idempotency_key: "idem_123"}

    {:ok, first} = Charge.create("acct_1", attrs)
    {:ok, second} = Charge.create("acct_1", attrs)

    assert first.id == second.id
  end
end
