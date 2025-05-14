use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Mint, Token, TokenAccount, Transfer, MintTo, Burn},
    associated_token::AssociatedToken,
};
use pyth_sdk_solana::load_price_feed_from_account_info;

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod solana4626 {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let admin = &mut ctx.accounts.admin;
        admin.authority = ctx.accounts.authority.key();
        Ok(())
    }

    pub fn create_asset(
        ctx: Context<CreateAsset>,
        name: String,
        ticker: String,
        price: u64,
        deposit_limit: u64,
    ) -> Result<()> {
        require!(name.len() <= 50, ErrorCode::NameTooLong);
        require!(ticker.len() <= 10, ErrorCode::TickerTooLong);
        
        let asset = &mut ctx.accounts.asset;
        asset.name = name;
        asset.ticker = ticker;
        asset.price = price;
        asset.mint = ctx.accounts.mint.key();
        asset.vault = ctx.accounts.vault.key();
        asset.authority = ctx.accounts.authority.key();

        let vault = &mut ctx.accounts.vault;
        vault.deposit_limit = deposit_limit;

        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        let asset = &ctx.accounts.asset;
        let vault = &mut ctx.accounts.vault;
        
        // Check if current deposit plus existing stablecoins would exceed limit
        let new_total = vault.total_usdc.checked_add(amount).unwrap();
        require!(
            new_total <= vault.deposit_limit,
            ErrorCode::DepositLimitExceeded
        );
        
        // Calculate asset tokens to mint based on USDC amount and price
        let asset_amount = amount
            .checked_mul(1_000_000) // Convert to 6 decimals
            .unwrap()
            .checked_div(asset.price)
            .unwrap();

        // Transfer USDC from user to vault
        let transfer_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.user_usdc_account.to_account_info(),
                to: ctx.accounts.vault_usdc_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::transfer(transfer_ctx, amount)?;

        // Mint asset tokens to user
        let mint_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.asset_mint.to_account_info(),
                to: ctx.accounts.user_asset_account.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
        );
        token::mint_to(mint_ctx, asset_amount)?;

        // Update vault state
        vault.total_usdc = new_total;
        vault.total_assets = vault.total_assets.checked_add(asset_amount).unwrap();

        Ok(())
    }

    pub fn redeem(ctx: Context<Redeem>, amount: u64) -> Result<()> {
        let asset = &ctx.accounts.asset;
        let vault = &mut ctx.accounts.vault;

        // Calculate USDC amount based on asset tokens and price
        let usdc_amount = amount
            .checked_mul(asset.price)
            .unwrap()
            .checked_div(1_000_000) // Convert from 6 decimals
            .unwrap();

        // Burn asset tokens
        let burn_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Burn {
                mint: ctx.accounts.asset_mint.to_account_info(),
                from: ctx.accounts.user_asset_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::burn(burn_ctx, amount)?;

        // Transfer USDC from vault to user
        let seeds = &[
            b"vault".as_ref(),
            asset.mint.as_ref(),
            &[ctx.bumps.vault],
        ];
        let signer = &[&seeds[..]];

        let transfer_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.vault_usdc_account.to_account_info(),
                to: ctx.accounts.user_usdc_account.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            signer,
        );
        token::transfer(transfer_ctx, usdc_amount)?;

        // Update vault state
        vault.total_usdc = vault.total_usdc.checked_sub(usdc_amount).unwrap();
        vault.total_assets = vault.total_assets.checked_sub(amount).unwrap();

        Ok(())
    }

    pub fn admin_withdraw(ctx: Context<AdminWithdraw>, amount: u64) -> Result<()> {
        let admin = &ctx.accounts.admin;
        let vault = &mut ctx.accounts.vault;

        // Verify admin authority
        require!(
            admin.authority == ctx.accounts.authority.key(),
            ErrorCode::Unauthorized
        );

        // Transfer USDC from vault to admin
        let seeds = &[
            b"vault".as_ref(),
            ctx.accounts.asset.mint.as_ref(),
            &[ctx.bumps.vault],
        ];
        let signer = &[&seeds[..]];

        let transfer_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.vault_usdc_account.to_account_info(),
                to: ctx.accounts.admin_usdc_account.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            signer,
        );
        token::transfer(transfer_ctx, amount)?;

        // Update vault state
        vault.total_usdc = vault.total_usdc.checked_sub(amount).unwrap();

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + Admin::LEN,
        seeds = [b"admin"],
        bump
    )]
    pub admin: Account<'info, Admin>,
    
    #[account(mut)]
    pub authority: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateAsset<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + Asset::LEN,
        seeds = [b"asset", mint.key().as_ref()],
        bump
    )]
    pub asset: Account<'info, Asset>,
    
    #[account(
        init,
        payer = authority,
        space = 8 + Vault::LEN,
        seeds = [b"vault", mint.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, Vault>,
    
    #[account(mut)]
    pub mint: Account<'info, Mint>,
    
    #[account(mut)]
    pub authority: Signer<'info>,
    
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(
        seeds = [b"asset", asset.mint.as_ref()],
        bump,
    )]
    pub asset: Account<'info, Asset>,
    
    #[account(
        mut,
        seeds = [b"vault", asset.mint.as_ref()],
        bump,
    )]
    pub vault: Account<'info, Vault>,
    
    #[account(mut)]
    pub asset_mint: Account<'info, Mint>,
    
    #[account(mut)]
    pub user_usdc_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub vault_usdc_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub user_asset_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub user: Signer<'info>,
    
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(
        seeds = [b"asset", asset.mint.as_ref()],
        bump,
    )]
    pub asset: Account<'info, Asset>,
    
    #[account(
        mut,
        seeds = [b"vault", asset.mint.as_ref()],
        bump,
    )]
    pub vault: Account<'info, Vault>,
    
    #[account(mut)]
    pub asset_mint: Account<'info, Mint>,
    
    #[account(mut)]
    pub user_usdc_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub vault_usdc_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub user_asset_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub user: Signer<'info>,
    
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct AdminWithdraw<'info> {
    #[account(
        seeds = [b"admin"],
        bump,
    )]
    pub admin: Account<'info, Admin>,
    
    #[account(
        seeds = [b"asset", asset.mint.as_ref()],
        bump,
    )]
    pub asset: Account<'info, Asset>,
    
    #[account(
        mut,
        seeds = [b"vault", asset.mint.as_ref()],
        bump,
    )]
    pub vault: Account<'info, Vault>,
    
    #[account(mut)]
    pub vault_usdc_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub admin_usdc_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub authority: Signer<'info>,
    
    pub token_program: Program<'info, Token>,
}

#[account]
pub struct Admin {
    pub authority: Pubkey,
}

impl Admin {
    pub const LEN: usize = 32; // authority (Pubkey)
}

#[account]
pub struct Asset {
    pub name: String,
    pub ticker: String,
    pub price: u64,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub authority: Pubkey,
}

impl Asset {
    pub const LEN: usize = 50 + 10 + 8 + 32 + 32 + 32; // name (String) + ticker (String) + price (u64) + mint (Pubkey) + vault (Pubkey) + authority (Pubkey)
}

#[account]
pub struct Vault {
    pub total_usdc: u64,
    pub total_assets: u64,
    pub deposit_limit: u64,
}

impl Vault {
    pub const LEN: usize = 8 + 8 + 8; // total_usdc (u64) + total_assets (u64) + deposit_limit (u64)
}

#[error_code]
pub enum ErrorCode {
    #[msg("Name is too long")]
    NameTooLong,
    #[msg("Ticker is too long")]
    TickerTooLong,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Deposit would exceed limit")]
    DepositLimitExceeded,
}
