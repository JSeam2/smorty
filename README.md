# Smorty 🤔

Smorty is a Smart Indexer which allows you to index events on the EVM easily.

## How it works
1. Copy `config.yaml.example` to `config.yaml`

2. Fill in the chains
```yaml
# RPC endpoints keyed by EVM chain id
chains:
  mainnet: "REPLACE_WITH_YOUR_MAINNET_RPC_URL"
  sonic: "REPLACE_WITH_YOUR_SONIC_RPC_URL"
```

3. Specify AI provider to use. OpenAI supported atm
```yaml
# External AI provider, TODO: support local LLMs
ai:
  openai:
    model: "gpt-5-2025-08-07"
    apiKey: "REPLACE_WITH_YOUR_OPENAI_API_KEY"
    temperature: 0.0
```

4. Write your specifications.
```yaml
# Define contracts and specifications
contracts:
  FeeManagerV3_Beets_Sonic_ETHUSD6h:
    chain: sonic
    address: "0x3295c142F1D0A2627A8a02Caedb1C5739A68Dd30"
    abiPath: "abi/FeeManagerV3_Beets.json"

    specs:
      - name: FeeUpdated
        startBlock: 47463429
        endpoint: "/sonic/FeeManagerV3Beets-Sonic-ETHUSD6h/FeeUpdated"
        task: |
          1. Track event FeeUpdated(uint256 swapFeePercentage).
          2. Persist the swapFeePercentage field along with the blockNumber and timestamp to return a time series of fee updates.
          3. Allow a user to specify limit, offset, startBlockNumber, endBlockNumber, startTimestamp, endTimestamp as query parameters to filter the results.

      - name: PoolUpdated
        startBlock: 47463429
        endpoint: "/sonic/FeeManagerV3Beets_Sonic_ETHUSD6h/PoolUpdated"
        task: |
          1. Track event PoolUpdated(address indexed pool, uint256 swapFeePercentage).
          2. Persist the swapFeePercentage field along with the blockNumber and timestamp to return a time series of fee updates.
          3. Allow a user to specify limit, offset, startBlockNumber, endBlockNumber, startTimestamp, endTimestamp as query parameters to filter the results.
```