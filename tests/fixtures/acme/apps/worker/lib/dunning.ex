defmodule AcmeWorker.Dunning do
  @moduledoc """
  Oban worker that retries failed invoice payments (dunning). NOT idempotent
  per-invoice: re-running a partially-failed batch re-emails already-charged
  customers. See the worker hazard.
  """

  use Oban.Worker, queue: :dunning, max_attempts: 3

  require Logger

  @batch_size 200

  @impl Oban.Worker
  def perform(%Oban.Job{args: %{"batch_id" => batch_id}}) do
    run_batch(batch_id)
  end

  # stele:landmark dunning-batch
  def run_batch(batch_id) do
    batch_id
    |> load_overdue_invoices()
    |> Enum.each(&retry_payment/1)
  end

  defp load_overdue_invoices(batch_id) do
    Logger.info("dunning batch #{batch_id}, up to #{@batch_size} invoices")
    []
  end

  defp retry_payment(_invoice) do
    :ok
  end
end
