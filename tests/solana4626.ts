import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Solana4626 } from "../target/types/solana4626";
import { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { assert } from "chai";

describe("solana4626", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Solana4626 as Program<Solana4626>;
  
  let admin: PublicKey;
  let adminBump: number;
  let usdcMint: PublicKey;
  let assetMint: PublicKey;
  let userUsdcAccount: PublicKey;
  let adminUsdcAccount: PublicKey;
  let userAssetAccount: PublicKey;
  let vaultUsdcAccount: PublicKey;
  let asset: PublicKey;
  let assetBump: number;
  let vault: PublicKey;
  let vaultBump: number;

  before(async () => {
    // Find admin PDA
    [admin, adminBump] = await PublicKey.findProgramAddress(
      [Buffer.from("admin")],
      program.programId
    );

    // Create USDC mint (for testing)
    usdcMint = await createMint(
      provider.connection,
      provider.wallet.payer,
      provider.wallet.publicKey,
      null,
      6 // USDC has 6 decimals
    );

    // Create asset mint
    assetMint = await createMint(
      provider.connection,
      provider.wallet.payer,
      provider.wallet.publicKey,
      null,
      9
    );

    // Create user USDC account
    userUsdcAccount = await createAccount(
      provider.connection,
      provider.wallet.payer,
      usdcMint,
      provider.wallet.publicKey
    );

    // Create admin USDC account
    adminUsdcAccount = await createAccount(
      provider.connection,
      provider.wallet.payer,
      usdcMint,
      provider.wallet.publicKey
    );

    // Mint some USDC to the user
    await mintTo(
      provider.connection,
      provider.wallet.payer,
      usdcMint,
      userUsdcAccount,
      provider.wallet.publicKey,
      1_000_000_000 // 1000 USDC
    );

    // Find asset PDA
    [asset, assetBump] = await PublicKey.findProgramAddress(
      [Buffer.from("asset"), assetMint.toBuffer()],
      program.programId
    );

    // Find vault PDA
    [vault, vaultBump] = await PublicKey.findProgramAddress(
      [Buffer.from("vault"), assetMint.toBuffer()],
      program.programId
    );

    // Create vault USDC account
    vaultUsdcAccount = await createAccount(
      provider.connection,
      provider.wallet.payer,
      usdcMint,
      vault,
      true // allowOwnerOffCurve
    );

    // Create user asset account
    userAssetAccount = await createAccount(
      provider.connection,
      provider.wallet.payer,
      assetMint,
      provider.wallet.publicKey
    );
  });

  it("Initializes the admin", async () => {
    await program.methods
      .initialize()
      .accounts({
        admin,
        authority: provider.wallet.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const adminAccount = await program.account.admin.fetch(admin);
    assert.ok(adminAccount.authority.equals(provider.wallet.publicKey));
  });

  it("Creates a new asset", async () => {
    const name = "Test Asset";
    const ticker = "TEST";
    const price = new anchor.BN(1_000_000); // 1 USDC per asset token
    const depositLimit = new anchor.BN(1_000_000_000); // 1000 USDC deposit limit

    await program.methods
      .createAsset(name, ticker, price, depositLimit)
      .accounts({
        asset,
        vault,
        mint: assetMint,
        authority: provider.wallet.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const assetAccount = await program.account.asset.fetch(asset);
    assert.equal(assetAccount.name, name);
    assert.equal(assetAccount.ticker, ticker);
    assert.equal(assetAccount.price.toNumber(), price.toNumber());
    assert.ok(assetAccount.mint.equals(assetMint));
    assert.ok(assetAccount.vault.equals(vault));
    assert.ok(assetAccount.authority.equals(provider.wallet.publicKey));

    const vaultAccount = await program.account.vault.fetch(vault);
    assert.equal(vaultAccount.depositLimit.toNumber(), depositLimit.toNumber());
  });

  it("Deposits USDC and receives asset tokens", async () => {
    const depositAmount = new anchor.BN(100_000); // 0.1 USDC

    await program.methods
      .deposit(depositAmount)
      .accounts({
        asset,
        vault,
        assetMint,
        userUsdcAccount,
        vaultUsdcAccount,
        userAssetAccount,
        user: provider.wallet.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    const vaultAccount = await program.account.vault.fetch(vault);
    assert.equal(vaultAccount.totalUsdc.toNumber(), depositAmount.toNumber());
    assert.equal(vaultAccount.totalAssets.toNumber(), depositAmount.toNumber());

    const userAssetBalance = await getAccount(provider.connection, userAssetAccount);
    assert.equal(userAssetBalance.amount, depositAmount.toNumber());
  });

  it("Fails when deposit would exceed limit with existing stablecoins", async () => {
    // First deposit 900 USDC (leaving 100 USDC capacity)
    const firstDeposit = new anchor.BN(900_000_000);
    await program.methods
      .deposit(firstDeposit)
      .accounts({
        asset,
        vault,
        assetMint,
        userUsdcAccount,
        vaultUsdcAccount,
        userAssetAccount,
        user: provider.wallet.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    // Try to deposit 200 USDC (would exceed 1000 USDC limit)
    const secondDeposit = new anchor.BN(200_000_000);
    try {
      await program.methods
        .deposit(secondDeposit)
        .accounts({
          asset,
          vault,
          assetMint,
          userUsdcAccount,
          vaultUsdcAccount,
          userAssetAccount,
          user: provider.wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();
      assert.fail("Expected deposit to fail due to limit");
    } catch (err) {
      assert.include(err.message, "DepositLimitExceeded");
    }

    // Verify vault state hasn't changed
    const vaultAccount = await program.account.vault.fetch(vault);
    assert.equal(vaultAccount.totalUsdc.toNumber(), firstDeposit.toNumber());
  });

  it("Redeems asset tokens for USDC", async () => {
    const redeemAmount = new anchor.BN(50_000); // 0.05 asset tokens

    await program.methods
      .redeem(redeemAmount)
      .accounts({
        asset,
        vault,
        assetMint,
        userUsdcAccount,
        vaultUsdcAccount,
        userAssetAccount,
        user: provider.wallet.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const vaultAccount = await program.account.vault.fetch(vault);
    assert.equal(vaultAccount.totalUsdc.toNumber(), 50_000); // 0.05 USDC remaining
    assert.equal(vaultAccount.totalAssets.toNumber(), 50_000); // 0.05 asset tokens remaining
  });

  it("Admin withdraws USDC from vault", async () => {
    const withdrawAmount = new anchor.BN(25_000); // 0.025 USDC

    await program.methods
      .adminWithdraw(withdrawAmount)
      .accounts({
        admin,
        asset,
        vault,
        vaultUsdcAccount,
        adminUsdcAccount,
        authority: provider.wallet.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const vaultAccount = await program.account.vault.fetch(vault);
    assert.equal(vaultAccount.totalUsdc.toNumber(), 25_000); // 0.025 USDC remaining
  });
});
