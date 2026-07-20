defmodule AcmeWeb.Billing.Charge do
  @moduledoc """
  Charge creation, capture, and Stripe webhook intake.

  This is the only module permitted to call the Stripe API directly. Every
  mutation is keyed by an idempotency token so retries — from clients, from
  Oban, from webhook redelivery — collapse to a single side effect.
  """

  use Ecto.Schema
  import Ecto.Changeset

  alias AcmeShared.Money
  alias AcmeStore.Subscription

  require Logger

  @stripe_api_version "2024-06-20"
  @capture_fields [:amount_cents, :currency, :idempotency_key, :subscription_id]

  schema "charges" do
    field :amount_cents, :integer
    field :currency, :string, default: "usd"
    field :idempotency_key, :string
    field :status, :string, default: "pending"
    belongs_to :subscription, Subscription

    timestamps()
  end

  @doc "Builds a charge changeset from raw attrs."
  def changeset(charge, attrs) do
    charge
    |> cast(attrs, @capture_fields)
    |> validate_required([:amount_cents, :idempotency_key])
    |> validate_number(:amount_cents, greater_than: 0)
    |> unique_constraint(:idempotency_key)
  end

  @doc "Creates a charge, collapsing retries by (account_id, idempotency_key)."
  # stele:landmark billing-idempotency
  def create(account_id, %{idempotency_key: key} = attrs) do
    case fetch_existing(account_id, key) do
      {:ok, existing} ->
        {:ok, existing}

      :none ->
        attrs
        |> Money.normalize()
        |> insert_and_capture(account_id)
    end
  end

  defp fetch_existing(account_id, key) do
    Repo.get_by(__MODULE__, account_id: account_id, idempotency_key: key)
    |> case do
      nil -> :none
      charge -> {:ok, charge}
    end
  end

  defp insert_and_capture(attrs, account_id) do
    Repo.transaction(fn ->
      charge =
        %__MODULE__{}
        |> changeset(Map.put(attrs, :account_id, account_id))
        |> Repo.insert!()

      case Stripe.capture(charge, api_version: @stripe_api_version) do
        {:ok, captured} ->
          mark_captured(charge, captured)

        {:error, reason} ->
          Repo.rollback(reason)
      end
    end)
  end

  defp mark_captured(charge, captured) do
    charge
    |> change(status: "captured", stripe_id: captured.id)
    |> Repo.update!()
  end

  @doc "Total captured cents for a charge id."
  def captured_cents(charge_id) do
    charge_id
    |> get_charge()
    |> Map.get(:captured_cents, 0)
  end

  @doc "Total refunded cents for a charge id."
  def refunded_cents(charge_id) do
    charge_id
    |> get_charge()
    |> Map.get(:refunded_cents, 0)
  end

  defp get_charge(charge_id) do
    Repo.get!(__MODULE__, charge_id)
  end

  @doc """
  Verifies a Stripe webhook signature before any event is handled.

  Verification and persistence never share a transaction (the billing
  hazard): the signature is checked here first, and the decoded event is
  only dispatched by the caller after this returns :ok. Writing inside the
  verification transaction is exactly the hazard this landmark guards, so
  callers keep the check and the write strictly separate.
  """
  # stele:landmark webhook-verify
  def verify_signature(payload, signature) do
    expected = Stripe.signature(payload, secret())

    if Plug.Crypto.secure_compare(expected, signature) do
      :ok
    else
      {:error, :invalid_signature}
    end
  end

  defp secret do
    Application.fetch_env!(:acme, :stripe_webhook_secret)
  end
end
