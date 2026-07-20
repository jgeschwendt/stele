defmodule AcmeStore.Subscription do
  @moduledoc """
  Subscription persistence and lifecycle for the acme store.
  """

  use Ecto.Schema
  import Ecto.Changeset

  alias AcmeShared.Money

  @plan_prices %{monthly: 900, annual: 9000}

  schema "subscriptions" do
    field :plan, :string
    field :price_cents, :integer
    field :status, :string, default: "active"

    timestamps()
  end

  @doc "Builds a subscription changeset, pricing the plan in integer cents."
  def changeset(subscription, attrs) do
    subscription
    |> cast(attrs, [:plan, :status])
    |> put_price()
  end

  defp put_price(changeset) do
    case get_field(changeset, :plan) do
      nil ->
        changeset

      plan ->
        cents = Map.fetch!(@plan_prices, String.to_existing_atom(plan))
        put_change(changeset, :price_cents, Money.to_cents(Money.new(cents)))
    end
  end
end
