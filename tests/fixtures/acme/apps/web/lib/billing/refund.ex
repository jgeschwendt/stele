defmodule AcmeWeb.Billing.Refund do
  @moduledoc """
  Refund changesets and cap enforcement.

  Refund amounts are validated against the remaining captured balance of the
  parent charge; the cap lives at the changeset layer so every write path —
  controller, console, background job — inherits it.
  """

  use Ecto.Schema
  import Ecto.Changeset

  alias AcmeWeb.Billing.Charge

  @castable_fields [:amount_cents, :charge_id]

  # Public API: the single cap-enforcing changeset every write path uses.
  # stele:landmark refund-cap
  # stele:claim apps/web/lib/billing/refund-cap
  @doc "Caps refund at remaining captured amount. See adr/0007 for integer-cents."
  def changeset(refund, attrs) do
    refund
    |> cast(attrs, @castable_fields)
    |> validate_refund_cap()
  end

  defp validate_refund_cap(changeset) do
    validate_change(changeset, :amount_cents, fn :amount_cents, amount ->
      if amount > remaining_cents(changeset) do
        [amount_cents: "exceeds remaining captured amount"]
      else
        []
      end
    end)
  end

  defp remaining_cents(changeset) do
    charge_id = get_field(changeset, :charge_id)
    Charge.captured_cents(charge_id) - Charge.refunded_cents(charge_id)
  end
end
