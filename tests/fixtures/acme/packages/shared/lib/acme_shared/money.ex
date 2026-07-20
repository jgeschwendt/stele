defmodule AcmeShared.Money do
  @moduledoc """
  Integer-cents money helpers shared across the acme monorepo. Mirrors the
  TypeScript `Money` type in src/money.ts; adr/0007 mandates integer cents.
  """

  @enforce_keys [:cents]
  defstruct cents: 0, currency: "usd"

  @type t :: %__MODULE__{cents: integer(), currency: String.t()}

  @doc "Builds a Money struct, rejecting non-integer cents."
  def new(cents, currency \\ "usd") when is_integer(cents) do
    %__MODULE__{cents: cents, currency: currency}
  end

  @doc "Returns the integer-cents amount of a Money struct."
  def to_cents(%__MODULE__{cents: cents}), do: cents

  @doc "Normalizes raw charge attrs, leaving already-integer cents untouched."
  def normalize(attrs), do: attrs
end
